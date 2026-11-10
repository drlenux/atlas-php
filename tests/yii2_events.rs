//! Yii2 Event::on / $obj->on / off; trigger by EVENT_* or string literal.
use code_atlas::{
    analyzer::{AnalysisOptions, SourceFile},
    domain::GraphSnapshot,
};

const YII: &str = r#"<?php
namespace yii\base;
class Component {
  public function trigger($name, $event = null) {}
  public function on($name, $handler) {}
  public function off($name, $handler = null) {}
  public static function on($class, $name, $handler) {}
}
class Event extends Component {
  public static function on($class, $name, $handler) {}
  public static function off($class, $name, $handler = null) {}
}
class Application extends Component {
  public const EVENT_AFTER_REQUEST = 'afterRequest';
}
"#;

const PAY: &str = r#"<?php
namespace common\components;
use yii\base\Component;
class PayPlace extends Component {
  public const EVENT_AFTER_SUCCESS_DEPOSIT = 'afterSuccessDepositPayPlace';
  public function triggerAfterSuccessDeposit($payment, $data = []) {
    $this->trigger(self::EVENT_AFTER_SUCCESS_DEPOSIT, null);
  }
  public function triggerByLiteral() {
    $this->trigger('afterSuccessDepositPayPlace', null);
  }
}
"#;

const HANDLE: &str = r#"<?php
namespace common\events;
use common\components\PayPlace;
use yii\base\Event;
class HandlePayPlace {
  public static function register(): void {
    Event::on(
      class: PayPlace::class,
      name: PayPlace::EVENT_AFTER_SUCCESS_DEPOSIT,
      handler: static fn(Event $event) => HandlePayPlace::onSuccessDeposit($event),
    );
  }
  public static function onSuccessDeposit(Event $event): void {}
}
"#;

const TRACE: &str = r#"<?php
namespace common\components\telemetry;
use yii\base\Event;
use yii\httpclient\Client;
class HttpClientTracing {
  public static function register(): void {
    self::unregister();
    Event::on(Client::class, Client::EVENT_BEFORE_SEND, [self::class, 'onBeforeSend']);
  }
  public static function unregister(): void {
    Event::off(Client::class, Client::EVENT_BEFORE_SEND, [self::class, 'onBeforeSend']);
  }
  public static function onBeforeSend($event): void {}
}
"#;

const CLIENT: &str = r#"<?php
namespace yii\httpclient;
use yii\base\Component;
class Client extends Component {
  public const EVENT_BEFORE_SEND = 'beforeSend';
  public function send() {
    $this->trigger(self::EVENT_BEFORE_SEND, null);
  }
}
"#;

const MODEL: &str = r#"<?php
namespace common\components\db;
use yii\base\Component;
class HasLoggableAttributes extends Component {
  public const EVENT_BEFORE_UPDATE = 'beforeUpdate';
  public function init() {
    $this->on(self::EVENT_BEFORE_UPDATE, [$this, 'logChanges']);
  }
  public function logChanges() {}
  public function save() {
    $this->trigger(self::EVENT_BEFORE_UPDATE, null);
  }
}
"#;

const LITERAL_SUB: &str = r#"<?php
namespace common\events;
use common\components\PayPlace;
use yii\base\Event;
class HandleLiteral {
  public static function register(): void {
    Event::on(PayPlace::class, 'afterSuccessDepositPayPlace', [self::class, 'onLiteral']);
  }
  public static function onLiteral($e): void {}
}
"#;

fn analyze(extra: &[(&str, &str)]) -> GraphSnapshot {
    let mut files = vec![
        ("vendor/yiisoft/yii2/base/Component.php", YII),
        ("common/components/PayPlace.php", PAY),
        ("common/events/HandlePayPlace.php", HANDLE),
        ("common/components/telemetry/HttpClientTracing.php", TRACE),
        // Not under declaration_paths — body must be analyzed for trigger().
        ("lib/yii/httpclient/Client.php", CLIENT),
        ("common/components/db/HasLoggableAttributes.php", MODEL),
        ("common/events/HandleLiteral.php", LITERAL_SUB),
    ];
    files.extend_from_slice(extra);
    let files: Vec<_> = files
        .into_iter()
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

#[test]
fn event_node_handles_named_on_handler() {
    let g = analyze(&[]);
    let event = g
        .nodes
        .iter()
        .find(|n| n.kind == "event" && n.qualified_name.contains("EVENT_AFTER_SUCCESS_DEPOSIT"))
        .expect("event node");
    assert!(g.edges.iter().any(|e| {
        e.kind == "handles"
            && e.from == event.id
            && e.to.as_deref() == Some("method:common\\events\\HandlePayPlace::onSuccessDeposit")
    }));
}

#[test]
fn trigger_calls_registered_handler() {
    let g = analyze(&[]);
    let calls: Vec<_> = g
        .edges
        .iter()
        .filter(|e| {
            e.kind == "calls"
                && e.to.as_deref()
                    == Some("method:common\\events\\HandlePayPlace::onSuccessDeposit")
                && e.from.contains("triggerAfterSuccessDeposit")
        })
        .collect();
    assert!(!calls.is_empty(), "{calls:?}");
    assert_eq!(calls[0].metadata["framework"], "yii2");
}

#[test]
fn positional_on_array_handler() {
    let g = analyze(&[]);
    assert!(g.edges.iter().any(|e| {
        e.kind == "handles"
            && e.to.as_deref()
                == Some("method:common\\components\\telemetry\\HttpClientTracing::onBeforeSend")
    }));
}

#[test]
fn instance_on_links_trigger_to_handler() {
    let g = analyze(&[]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref()
                    == Some("method:common\\components\\db\\HasLoggableAttributes::logChanges")
                && e.from.contains("HasLoggableAttributes::save")
                && e.metadata["framework"] == "yii2"
        }),
        "instance $this->on should link trigger; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls" && e.metadata["framework"] == "yii2")
            .map(|e| (e.from.clone(), e.to.clone(), e.metadata["event"].clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn off_records_unsubscribe_but_keeps_register_handler() {
    let g = analyze(&[]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "unsubscribes"
                && e.to.as_deref()
                    == Some("method:common\\components\\telemetry\\HttpClientTracing::onBeforeSend")
        }),
        "expected unsubscribes edge; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "unsubscribes")
            .collect::<Vec<_>>()
    );
    // register()+unregister() pair must still leave trigger→handler
    assert!(g.edges.iter().any(|e| {
        e.kind == "calls"
            && e.to.as_deref()
                == Some("method:common\\components\\telemetry\\HttpClientTracing::onBeforeSend")
            && e.from.contains("Client::send")
    }));
}

#[test]
fn off_only_still_creates_event_node() {
    const DEBUG: &str = r#"<?php
namespace yii\debug;
use yii\base\Event;
use yii\base\Application;
class Module {
  public const EVENT_END_BODY = 'endBody';
  public function renderToolbar() {
    Event::off(Application::class, self::EVENT_END_BODY, [$this, 'renderToolbar']);
  }
}
"#;
    let g = analyze(&[("vendor/yiisoft/yii2-debug/src/Module.php", DEBUG)]);
    let event_id = "event:yii\\base\\Application::EVENT_END_BODY";
    // Owner from Application::class in off(), not Module.
    assert!(
        g.nodes.iter().any(|n| n.id == event_id)
            || g.nodes
                .iter()
                .any(|n| { n.kind == "event" && n.qualified_name.contains("EVENT_END_BODY") }),
        "off-only must create event node; events={:?}",
        g.nodes
            .iter()
            .filter(|n| n.kind == "event")
            .map(|n| &n.id)
            .collect::<Vec<_>>()
    );
    let edge = g
        .edges
        .iter()
        .find(|e| e.kind == "unsubscribes")
        .expect("unsubscribes");
    assert!(
        g.nodes.iter().any(|n| n.id == edge.from),
        "unsubscribes.from must exist: {}",
        edge.from
    );
}

#[test]
fn string_literal_on_and_trigger_match_via_constant_value() {
    let g = analyze(&[]);
    // Event::on(..., 'afterSuccessDepositPayPlace', ...) → trigger(self::EVENT_*)
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\events\\HandleLiteral::onLiteral")
                && e.from.contains("triggerAfterSuccessDeposit")
        }),
        "literal subscription should match EVENT_* trigger by constant value"
    );
    // trigger('afterSuccessDepositPayPlace') → HandlePayPlace (subscribed via EVENT_*)
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref()
                    == Some("method:common\\events\\HandlePayPlace::onSuccessDeposit")
                && e.from.contains("triggerByLiteral")
        }),
        "literal trigger should match EVENT_* subscription"
    );
}

#[test]
fn trigger_owner_is_receiver_not_event_const_class() {
    // Event::on(RegistrationController, User::EVENT_*) + $this->trigger(User::EVENT_*)
    // must match on RegistrationController, not User.
    const USER: &str = r#"<?php
namespace common\models;
class User {
  public const EVENT_CONFIRM_PHONE_NUMBER = 'confirmPhoneNumber';
}
"#;
    const REG: &str = r#"<?php
namespace partner\controllers;
use common\models\User;
use yii\base\Component;
class RegistrationController extends Component {
  public function actionConfirmPhone() {
    $this->trigger(User::EVENT_CONFIRM_PHONE_NUMBER, null);
  }
}
"#;
    const HANDLER: &str = r#"<?php
namespace common\events;
use partner\controllers\RegistrationController;
use common\models\User;
use yii\base\Event;
class HandlePhone {
  public static function register(): void {
    Event::on(RegistrationController::class, User::EVENT_CONFIRM_PHONE_NUMBER, [self::class, 'onConfirm']);
  }
  public static function onConfirm($e): void {}
}
"#;
    let g = analyze(&[
        ("common/models/User.php", USER),
        ("partner/controllers/RegistrationController.php", REG),
        ("common/events/HandlePhone.php", HANDLER),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\events\\HandlePhone::onConfirm")
                && e.from.contains("actionConfirmPhone")
                && e.metadata["receiver_class"] == "partner\\controllers\\RegistrationController"
        }),
        "receiver owner must match cross-class EVENT_*; {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls" && e.metadata["framework"] == "yii2")
            .map(|e| (
                e.from.clone(),
                e.to.clone(),
                e.metadata["receiver_class"].clone()
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn event_static_trigger_links_handlers() {
    const TASK: &str = r#"<?php
namespace common\services;
use yii\base\Component;
use yii\base\Event;
class MakeDepositTask extends Component {
  public const EVENT_SUCCESS_DEPOSIT = 'successDeposit';
  public function tryComplete() {
    Event::trigger(static::class, self::EVENT_SUCCESS_DEPOSIT, null);
  }
  public function namedTrigger() {
    Event::trigger(class: __CLASS__, name: self::EVENT_SUCCESS_DEPOSIT, event: null);
  }
}
"#;
    const HANDLER: &str = r#"<?php
namespace common\events;
use common\services\MakeDepositTask;
use yii\base\Event;
class HandleDeposit {
  public static function register(): void {
    Event::on(MakeDepositTask::class, MakeDepositTask::EVENT_SUCCESS_DEPOSIT, [self::class, 'onDeposit']);
  }
  public static function onDeposit($e): void {}
}
"#;
    let g = analyze(&[
        ("common/services/MakeDepositTask.php", TASK),
        ("common/events/HandleDeposit.php", HANDLER),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\events\\HandleDeposit::onDeposit")
                && e.from.contains("tryComplete")
        }),
        "Event::trigger(static::class, …); {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls" && e.metadata["framework"] == "yii2")
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\events\\HandleDeposit::onDeposit")
                && e.from.contains("namedTrigger")
        }),
        "Event::trigger(class: __CLASS__, name: …)"
    );
}

#[test]
fn event_trigger_foreign_class_links_handlers() {
    // Event::trigger(Payment::class, Payment::EVENT_*) from another class must still
    // reach handlers registered on Payment (regression vs only static::class / $this).
    // Ambiguous short name `Payment` must prefer the owner that declares EVENT_*.
    const PAY: &str = r#"<?php
namespace common\models;
use yii\base\Component;
class Payment extends Component {
  public const EVENT_SUCCESS_DEPOSIT = 'successDeposit';
}
"#;
    const OTHER_PAY: &str = r#"<?php
namespace api\models;
use yii\base\Component;
class Payment extends Component {}
"#;
    const HANDLER: &str = r#"<?php
namespace common\events;
use common\models\Payment;
use yii\base\Event;
class HandlePay {
  public static function register(): void {
    Event::on(Payment::class, Payment::EVENT_SUCCESS_DEPOSIT, [self::class, 'onDeposit']);
  }
  public static function onDeposit($e): void {}
}
"#;
    const OTHER: &str = r#"<?php
namespace common\tests;
use common\models\Payment;
use yii\base\Event;
class TriggerElsewhere {
  public function fire($sender): void {
    Event::trigger(Payment::class, Payment::EVENT_SUCCESS_DEPOSIT, new Event(['sender' => $sender]));
  }
}
"#;
    let g = analyze(&[
        ("common/models/Payment.php", PAY),
        ("api/models/Payment.php", OTHER_PAY),
        ("common/events/HandlePay.php", HANDLER),
        ("common/tests/TriggerElsewhere.php", OTHER),
    ]);
    assert!(
        g.edges.iter().any(|e| {
            e.kind == "calls"
                && e.to.as_deref() == Some("method:common\\events\\HandlePay::onDeposit")
                && e.from.contains("TriggerElsewhere::fire")
                && e.metadata["framework_evidence"] == "event_trigger"
        }),
        "Event::trigger(Payment::class, …); {:?}",
        g.edges
            .iter()
            .filter(|e| e.kind == "calls")
            .map(|e| {
                (
                    e.from.clone(),
                    e.to.clone(),
                    e.metadata["framework_evidence"].clone(),
                    e.metadata["expression"].clone(),
                )
            })
            .collect::<Vec<_>>()
    );
}
