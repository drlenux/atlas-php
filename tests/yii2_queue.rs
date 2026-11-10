//! yii2-queue-dispatch: `queue->push(new Job)` links the call site to `Job::execute`.
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::{Edge, GraphSnapshot},
};

const VENDOR: &[(&str, &str)] = &[
    ("vendor/yiisoft/yii2/base/All.php", "<?php\nnamespace yii\\base;\nclass BaseObject {}\nclass Component extends BaseObject {}\nclass Module extends Component {}\nclass Application extends Module {}\n"),
    ("vendor/yiisoft/yii2/BaseYii.php", "<?php\nnamespace yii;\nclass BaseYii {\n  /** @var \\yii\\base\\Application */\n  public static $app;\n}\n"),
    ("vendor/yiisoft/yii2/Yii.php", "<?php\nclass Yii extends \\yii\\BaseYii {}\n"),
    ("vendor/yiisoft/yii2-queue/src/Queue.php", r#"<?php
namespace yii\queue;
interface JobInterface { public function execute($queue); }
abstract class Queue extends \yii\base\Component {
  public function push($job) { return $this->pushMessage($job); }
  /** @return $this */
  public function delay($value) { return $this; }
}
"#),
    ("vendor/yiisoft/yii2-queue/src/drivers/amqp_interop/Queue.php", "<?php\nnamespace yii\\queue\\amqp_interop;\nclass Queue extends \\yii\\queue\\Queue {}\n"),
];

const APP: &str = r#"<?php
namespace common\jobs;
use Yii;
use yii\base\BaseObject;
use yii\queue\JobInterface;
class GameProcess extends BaseObject implements JobInterface {
  public function execute($queue) {}
}
class NotAJob { public function run() {} }
class Producer {
  public function now() {
    Yii::$app->queueGame->push(new GameProcess(['action' => 1]));
  }
  public function later() {
    $job = new GameProcess();
    Yii::$app->queueGame->delay(5)->push($job);
  }
  public function unknown($payload) {
    Yii::$app->queueGame->push($payload);
    Yii::$app->queueGame->push(new NotAJob());
  }
}
"#;

fn analyze() -> GraphSnapshot {
    analyze_with(&[])
}
fn analyze_with(extra: &[(&str, &str)]) -> GraphSnapshot {
    let files: Vec<_> = VENDOR
        .iter()
        .copied()
        .chain(extra.iter().copied())
        .chain([
            ("common/jobs/GameProcess.php", APP),
            (
                "common/config/main.php",
                "<?php return ['components' => ['queueGame' => ['class' => \\yii\\queue\\amqp_interop\\Queue::class, 'queueName' => 'game']]];",
            ),
        ])
        .map(|(path, source)| SourceFile {
            path: path.into(),
            source: source.into(),
        })
        .collect();
    let options = AnalysisOptions {
        declaration_paths: vec!["vendor/yiisoft".into()],
        ..AnalysisOptions::default()
    };
    code_atlas::plugins::registry()
        .analyze(&files, &["php".into()], &["yii2".into()], &options)
        .unwrap()
}
fn dispatches(g: &GraphSnapshot) -> Vec<&Edge> {
    g.edges.iter().filter(|e| e.kind == "dispatches").collect()
}
fn owner_of<'a>(g: &'a GraphSnapshot, site: &str) -> &'a str {
    g.edges
        .iter()
        .find(|e| e.kind == "contains" && e.to.as_deref() == Some(site))
        .map(|e| e.from.as_str())
        .unwrap()
}

#[test]
fn push_of_a_job_links_the_call_site_to_execute_with_the_queue_name() {
    let g = analyze();
    let found = dispatches(&g);
    assert_eq!(found.len(), 2, "{found:#?}\n{:?}", g.diagnostics);
    let mut owners: Vec<_> = found.iter().map(|e| owner_of(&g, &e.from)).collect();
    owners.sort();
    assert_eq!(
        owners,
        [
            "method:common\\jobs\\Producer::later",
            "method:common\\jobs\\Producer::now"
        ]
    );
    for edge in found {
        assert_eq!(
            edge.to.as_deref(),
            Some("method:common\\jobs\\GameProcess::execute")
        );
        assert_eq!(edge.metadata["framework"], "yii2");
        assert_eq!(edge.metadata["framework_evidence"], "queue");
        assert_eq!(edge.metadata["queue"], "queueGame");
        assert_eq!(edge.metadata["job_class"], "common\\jobs\\GameProcess");
        // The queue is reached through Yii::$app, typed by vendor PHPDoc.
        assert_eq!(edge.metadata["resolution"], "annotated");
    }
}

#[test]
fn call_edges_record_argument_types_with_provenance() {
    let g = analyze();
    let push = g
        .edges
        .iter()
        .find(|e| {
            e.kind == "calls"
                && e.metadata["member_name"] == "push"
                && owner_of(&g, &e.from) == "method:common\\jobs\\Producer::now"
        })
        .unwrap();
    assert_eq!(
        push.metadata["arguments"][0]["type"],
        "common\\jobs\\GameProcess"
    );
    assert_eq!(push.metadata["arguments"][0]["type_source"], "code");
}

#[test]
fn unknown_payloads_and_classes_without_execute_are_not_linked() {
    let g = analyze();
    assert!(!dispatches(&g)
        .iter()
        .any(|e| owner_of(&g, &e.from) == "method:common\\jobs\\Producer::unknown"));
}

#[test]
fn push_on_a_queue_with_several_driver_classes_still_dispatches_the_job() {
    let g = analyze_with(&[
        (
            "vendor/yiisoft/yii2-queue/src/drivers/sync/Queue.php",
            "<?php\nnamespace yii\\queue\\sync;\nclass Queue extends \\yii\\queue\\Queue {\n  public function push($job) { return parent::push($job); }\n}\n",
        ),
        (
            "common/config/test.php",
            "<?php return ['components' => ['queueGame' => ['class' => \\yii\\queue\\sync\\Queue::class]]];",
        ),
    ]);
    let now = g
        .edges
        .iter()
        .find(|e| {
            e.kind == "calls"
                && e.metadata["member_name"] == "push"
                && owner_of(&g, &e.from) == "method:common\\jobs\\Producer::now"
        })
        .unwrap();
    assert_eq!(now.metadata["resolution"], "candidate", "{now:#?}");
    let found: Vec<_> = dispatches(&g)
        .into_iter()
        .filter(|e| e.from == now.from)
        .collect();
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        found[0].to.as_deref(),
        Some("method:common\\jobs\\GameProcess::execute")
    );
    // Yii::$app is typed by vendor PHPDoc even when push has several candidates.
    assert_eq!(found[0].metadata["resolution"], "annotated");
}
