//! Yii::createObject(X::class) types the result as X so instance calls resolve.
use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    domain::GraphSnapshot,
    plugins::php::PhpLanguagePlugin,
};

const SRC: &str = r#"<?php
namespace common\components;
class AdminPayment {
  public function deposit($userId, $amount, $currency) {}
}
namespace admin\controllers;
use common\components\AdminPayment as AdminPaymentComponent;
class Yii {
  public static function createObject($type, $params = []) { return null; }
}
class AdminPaymentController {
  public function actionDeposit() {
    $paymentComponent = Yii::createObject(AdminPaymentComponent::class);
    $payment = $paymentComponent->deposit(1, 100, 'UAH');
  }
}
"#;

fn analyze() -> GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "AdminPayment.php".into(),
                source: SRC.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

#[test]
fn create_object_class_string_types_receiver_for_deposit() {
    let g = analyze();
    let deposit = g
        .edges
        .iter()
        .find(|e| {
            e.kind == "calls"
                && e.metadata["member_name"] == "deposit"
                && e.from.contains("AdminPaymentController::actionDeposit")
        })
        .expect("deposit call edge");
    assert_eq!(
        deposit.to.as_deref(),
        Some("method:common\\components\\AdminPayment::deposit"),
        "receiver_class={:?} resolution={:?}",
        deposit.metadata["receiver_class"],
        deposit.metadata["resolution"]
    );
    assert_eq!(
        deposit.metadata["receiver_class"],
        "common\\components\\AdminPayment"
    );
    assert_ne!(deposit.metadata["resolution"], "unresolved");
    let variable = g
        .nodes
        .iter()
        .find(|n| n.id.contains("actionDeposit:$paymentComponent"))
        .expect("paymentComponent variable");
    assert_eq!(
        variable.metadata["type"],
        "common\\components\\AdminPayment"
    );
}
