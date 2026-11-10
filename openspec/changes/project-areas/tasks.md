# Tasks: project-areas

- [x] 1.1 Proposal, design, spec delta
- [x] 2.1 Red/green: `areas` in setup.yaml (parse, validate, longest-prefix lookup)
- [x] 3.1 Red/green: `metadata.area` stamping; presenter `[area]` labels; tree caller areas
- [x] 3.2 Red/green: `area` filter on search (memory + PostgreSQL), grep, outline, context
- [x] 4.1 Red/green: Yii components per area (`types_by_area`, enrich per path)
- [x] 5.1 Red/green: namespace-less names prefixed and resolved by area; global function fallback
- [x] 5.2 Red/green: area re-keying wins over rename matching for unprotected entities
- [x] 5.3 Red/green: a library global keeps its name over area copies (IDE stub `Yii`)
- [x] 6.1 Full checks; build; slotor areas; verify

## Evidence (2026-11-10)

TDD reds: areas config test (missing `area_of`/`areas`), registry labelling (no `area`),
presenter label tests, callers-tree area, HTTP area filter (unknown argument), PostgreSQL include
filter, Yii members (two `send` candidates), native PHP (one `Provider` overwrote the other and
`api()` was marked undeclared; namespaced fallback to `helper`; `common:Yii` stub shadowing the
library `Yii`), core_behavior re-keying (old id kept).

Checks: `cargo test --locked` 338 passed; `core_postgres` 9 passed and 1 pre-existing failure
(legacy cursor); clippy and fmt clean; compose contract 7 passed.

Live slotor (image 0.1.39, local) with areas partner, admin, api, grcallback, console, frontend,
backend and shared common:
- labels: common 189k, admin 78k, partner 39k, console 36k, api 25k, grcallback 7k nodes;
- prefixed namespace-less classes: console 455 (migrations), api 76 (swagger models), common 4;
  none left unprefixed inside areas;
- component members carry `types_by_area` (e.g. `urlManager`: admin
  `codemix\localeurls\UrlManager|yii\web\UrlManager`, api `yii\web\UrlManager`);
- `undeclared` back to 24 sites after two rollout regressions were fixed (old ids kept by rename
  matching; the `common/config/__autocomplete.php` stub `Yii` shadowing `Yii::t` → 5 407 false
  undeclared). `UserGame.php:83` left the list: `use Provider;` in common no longer reaches
  `api:Provider`, so the class is now reported as not found instead.
