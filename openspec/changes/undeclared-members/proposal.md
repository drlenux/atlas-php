# Proposal: undeclared-members

## Why

A systematic check of the 5 786 unresolved slotor call sites with a known receiver type found about
30 calls to methods that exist nowhere in the receiver's hierarchy (removed in refactorings,
e.g. `Wallet::checkCreateWager`, `Session::delete`, `FinanceController::checkPaymentSystemData`).
Atlas knew the receiver type but reported the same generic `unresolved_call` as for unknown
receivers. The same check showed atlas typing errors (95 false `yii\console\Request::post`) and the
owner benchmark showed missed callers through untyped variables and noisy `atlas_grep` output
(tests, `.idea`).

## What changes

- Property reads on a union receiver take the union of the property types of every part
  (`Yii::$app->request` → `yii\console\Request|yii\web\Request`), not the first part's type.
- Calls whose receiver classes are fully indexed and declare the method nowhere (class, ancestors,
  traits, descendants, `__call`, Yii behaviors) get `undeclared: true` and the diagnostic
  `undeclared_member` instead of `unresolved_call`. The PHP plugin decides for plain classes; the
  Yii2 plugin decides for components whose only magic is `Component::__call` (behaviors).
- Outline `called_by` and the callers tree list unresolved call sites with the same member name
  and an unknown receiver as `[unresolved: name only]` when the method name is declared in at most
  three classes; otherwise only their count is reported.
- `atlas_grep` leaves out test paths unless `include_tests` (count `excluded.tests`); `.idea` and
  `.vscode` join the always-skipped tool directories.
