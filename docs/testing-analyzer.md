> Historical verification of the pre-v0.4 implementation. PHP subprocess / Git CLI references below are obsolete. Current native-plugin evidence is recorded in `openspec/changes/add-code-atlas/verification.md` and the native testing reports.

# Analyzer TDD evidence

Implementation branch: `feature/atlas-analyzer`; isolated Git worktree. Architecture and the GraphSnapshot contract were read from the committed OpenSpec `design.md` before implementation.

## Red and green sequence

1. Added AST fixtures and subprocess behavior tests before production code. Running `php -d xdebug.mode=off plugins/php/tests/run.php` failed because the analyzer entry point did not exist. This established the missing observable behavior.
2. Implemented isolated AST parsing, declaration/name passes, scope analysis and Yii enrichment. The original fixture suite passed **132 checks**.
3. Added cases for trait alias/precedence, callee `self`, `unset`, branch assignment alternatives, NUL paths and escaping symlinks. The test failed at **“Callee self refers to its defining class”**. Corrected type context, binding invalidation and path validation; **144 checks passed**.
4. Added class-polymorphism and conflicting-component tests. The test failed at **“Class-typed parameter does not prove active override”**. Distinguished exact constructed receivers from type constraints and guarded conflicting Yii component maps; **152 checks passed**.
5. Added the requirement that an indexed component definition is a candidate, not evidence of active runtime configuration. The test failed at **“Indexed component configuration is not proof of active runtime configuration”**. Corrected component call evidence; the final suite passes **153 checks**.

## Behavioral coverage

- Functions, classes, interfaces, traits, methods, properties, variables and typed relations.
- Parent constructor call versus inheritance; overriding without a false parent call; trait aliases and precedence.
- Multiple return types, individual source locations, nested if conditions, early-exit conditions and value origins beyond local snippets.
- Separate nested named-function, closure and arrow scopes; implicit completion and `unset` binding invalidation.
- Interface declaration calls with candidates and exact `new` receiver calls; subclass overrides remain candidates for constrained receivers.
- Yii getter/setter definitions in vendor, literal behaviors and components, dynamic invocation and conflicting configuration diagnostics.
- Rejected traversal/NUL/escaping symlink paths; parse errors do not emit partial graph; source PHP is never executed.
- Stable IDs under line shifts and structural fingerprints under class rename.

## Reproduction

```sh
php -d xdebug.mode=off plugins/php/tests/run.php
composer --working-dir=plugins/php validate --no-check-publish --no-check-all
```

Own PHP source and test files also pass `php -l`. The PHP runtime in this environment is 8.5.12; parser target used by fixtures is PHP 8.2. The Composer lock pins `nikic/php-parser` 5.9.0. `composer validate --strict` reports an intentional exact-version-constraint warning; `--no-check-all` disables that recommendation while still checking manifest/lock consistency.

These are analyzer checks, not a claim that Docker startup or the complete application was tested. Parser limitations and diagnostic meanings are documented in `plugins/php/README.md`.
