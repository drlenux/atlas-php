# Tasks: undeclared-members

- [x] 1.1 Proposal, design, spec delta
- [x] 2.1 Red/green: union receiver property type
- [x] 2.2 Red/green: PHP plugin `undeclared` + `undeclared_member`
- [x] 2.3 Red/green: Yii2 plugin `undeclared` for components (behaviors considered)
- [x] 2.4 Red/green: no `undeclared` when a declarer may be hidden (ancestry outside the index)
      or a descendant takes the method from a trait
- [x] 3.1 Red/green: name-only callers in outline and tree, with noise bound; presenter counts
- [x] 4.1 Red/green: atlas_grep test exclusion (MCP wiring shown to fail without it); IDE
      directories skipped
- [x] 5.1 Full checks; build; deploy; compare with the manual slotor list

## Evidence (2026-11-10)

TDD reds: `union_receiver_property_takes_every_part_type` (unresolved `post`),
`calls_to_methods_declared_nowhere_are_undeclared` (metadata without `undeclared`, later the
hidden-descendant case), `component_calls_missing_everywhere_including_behaviors_are_undeclared`,
`undeclared_needs_every_possible_declarer_to_be_ruled_out` (`getServer` marked),
`name_only_callers` (empty `called_by`, count 0), presenter count test, grep scope tests.

Checks: `cargo test --locked` 330 passed; clippy and fmt clean.

Live slotor (image 0.1.35, local): 25 call sites marked `undeclared`. Every removed-method call
of the manual analysis is among them (`Wallet::checkCreateWager/setWager/withdrawWager/
setPaymentTax`, `TaxUser::getDataForAnalyticByCurrency`, `FinanceController::
checkPaymentSystemData`, `PaymentSystem::getSumWithdrawPiasrtix/getCommissionPiasrtix`,
notify strategies, `ContactForm::sendEmail`, `Provider::className`, template forms, OFF/test
actions). Two first-run false positives were fixed (`Module::getServer`, `Query::with`). One
flagged site follows a wrong project annotation (`BonusItem` `@var BonusCodLog`) and is kept:
the annotation itself is the defect. Not flagged by design (unresolvable behaviors on the
receiver): `Session::delete`, `User::setPassword`/`getUnitsBalance` family.
`yii\console\Request` unresolved call sites: 95 → 0.
