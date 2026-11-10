## ADDED Requirements

### Requirement: Union receivers type properties from every part
A property read on a receiver typed as a union SHALL be typed as the union of the property types
of every part that declares the property.

#### Scenario: Yii application request
- **WHEN** `$app` is typed `ConsoleApp|WebApp` and each declares `@property` `request` with its own
  request class, and code calls `$app->request->post()` that only the web request declares
- **THEN** the call resolves to the web request's `post`

### Requirement: Calls to undeclared members are marked
A call whose receiver classes are fully indexed SHALL be marked `undeclared` with the diagnostic
`undeclared_member` when no class, ancestor, trait, descendant, `__call` or attached Yii behavior
declares the method.

#### Scenario: Removed method
- **WHEN** `$wallet->checkCreateWager()` is called on `Wallet` and no class declares it
- **THEN** the call site is `undeclared` and the diagnostic code is `undeclared_member`

#### Scenario: Magic call
- **WHEN** the receiver class declares `__call`
- **THEN** the call is not marked `undeclared`

#### Scenario: Yii behavior method
- **WHEN** a Yii component attaches a behavior whose class declares the method
- **THEN** the call is not marked `undeclared`

### Requirement: Name-only callers are offered
Outline `called_by` and the callers tree SHALL list unresolved call sites with an unknown receiver
and the same member name as possible callers marked `unresolved: name only`, when at most three
declarations share that method name, and SHALL otherwise report their count.

#### Scenario: Untyped variable
- **WHEN** `$w->rareMethod()` has an unknown receiver and only `A::rareMethod` exists
- **THEN** `called_by` of `A::rareMethod` lists that site marked `unresolved: name only`

### Requirement: Grep leaves out tests and tool directories
`atlas_grep` SHALL leave out test paths unless `include_tests` is true and report
`excluded.tests`; `.idea` and `.vscode` SHALL never be scanned.

#### Scenario: Test match
- **WHEN** the only match is under `common/tests`
- **THEN** no hit is returned and `excluded.tests=1` is reported
