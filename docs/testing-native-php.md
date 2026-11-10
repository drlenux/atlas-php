# Native PHP analyzer: evidence and coverage

The native implementation is `src/plugins/php`, compiled into Atlas through the language-neutral `LanguagePlugin` contract. It reads `SourceFile` text and never invokes PHP, Composer, project autoloaders, or source code. Yii2 enrichment is a separate framework plugin. Architecture was recorded in `docs/native-analyzer-design.md` before this implementation.

## Observed TDD sequence

1. Added native behavior tests before the plugin module existed. `cargo test --offline --test native_php` failed to compile because `code_atlas::plugins` was missing. Implemented tokenization, structural declarations, expression trees, scope flow and graph construction; nine native tests passed.
2. Added UTF-8 comments, identifiers, quoted strings and heredoc regression coverage and invalid `<?phpword` opening tags. The existing lexer panicked on a UTF-8 block-comment byte boundary. Byte-slice matching and a proper opening-tag boundary fixed the regression. Added a child-test process with an empty `PATH`; it analyzes the real fixture without a language runtime.
3. Migrated two previously runtime-dependent `core_analyzer` tests to direct native calls. Both run normally, without `ignore`, covering candidate context and persistent identity/trust after a real class rename.
4. Added target-version tests before version gates. Old targets accepted newer constructs. Gates now reject arrow functions before 7.4; nullsafe access, match, attributes and union types before 8.0; enums and readonly properties before 8.1; readonly classes before 8.2. A second red check exposed nullable `?int` being mistaken for a union; gates now inspect declaration syntax. A third exposed property unions being accepted at 7.4; property syntax is now included. Version comes from `AnalysisOptions.language_versions["php"]`, defaulting to 8.2.
5. Added malformed expression regressions (`$value = ;`, `return 1 + ;`). Parse-time defects still invalidate the whole file. Body-level defects and alternative control-flow keep class/method declarations and record `analysis_limitation` on the affected method (dropping the whole file hid models like `BonusCod` from the index).

Final checks in the native PHP worktree: **14 native tests + 2 core integration tests passed**, zero ignored. The relevant library and these two test targets also pass Clippy with warnings denied.

```sh
cargo test --offline --test native_php --test core_analyzer
cargo clippy --offline --lib --test native_php --test core_analyzer -- -D warnings
```

The shared `AnalysisOptions` change was coordinated with the integration owner; the PHP follow-up commit does not independently own that common contract.

## Historical assertion migration

The former PHP process suite reported 153 assertions, including repeated per-node invariants. That number is historical evidence for a different implementation, not a native test count. The behavioral groups map as follows:

| Historical behavior | Native evidence |
| --- | --- |
| Graph shape, trust, revision, source spans and structural fingerprints for every node | `real_fixture_has_typed_declarations_and_relations_without_runtime` |
| Inheritance, trait usage/availability, explicit parent constructor call, override without a false call | Same fixture test |
| Individual return cases, heterogeneous types including `resource|false`, nested and early-return conditions, snippets and nonlocal assignment origins | `return_cases_conditions_variables_and_origins_survive` |
| Variable assignment/use spans and inferred types | Same return/variable test |
| Interface declaration plus multiple candidates; exact constructed receiver | `interface_candidates_do_not_become_active_implementation` |
| Nested named functions, closures/arrows, separate returns and implicit completion | `nested_functions_closures_and_implicit_completion_are_separate` |
| Trait alias/precedence, callee `self`, `unset`, merged assignment origins | `aliases_self_unset_and_branch_merges` |
| Class-constrained receiver versus exact constructed receiver | `constrained_receivers_preserve_subclass_candidates` |
| Stable identity under line shifts and structural matching under rename | `stable_ids_fingerprints_syntax_errors_and_inert_source`; both `core_analyzer` tests exercise actual service reconciliation |
| Source nonexecution and absence of PHP runtime dependency | `native_analysis_runs_without_runtime_on_path`; inert executable-looking source in the stable-ID test |
| Traversal/NUL paths and syntax failures without partial graph | `native_source_paths_must_stay_relative`; `incomplete_or_unsupported_files_do_not_publish_partial_declarations` |
| Filesystem symlink containment | Moved to source-loading/project boundary tests: the native plugin accepts text and never follows filesystem paths |
| Yii vendor getter/setter, behaviors, configured component candidates, conflicts and dynamic invocation | Separate `native_yii2` suite; PHP test `framework_hooks_are_structured_but_php_does_not_apply_yii` checks the unresolved evidence contract. Yii tests are not counted in the 16 PHP/core results above |

## Expression semantics (PLUGIN-05–PLUGIN-07)

Observed red before the fix: `(int)Yii::$app->request->post(...)` lost the `post` call and emitted seven fragment `analysis_limitation` messages; ternaries, `?:`, `$data['id']` and nested `(object)[...]` also collapsed into Unknown cascades. Permanent regressions in `tests/native_php.rs` now cover casts, ternary/elvis/`??`, index/`+=`, nested object-cast arrays, unresolved-vs-parse separation and 1/4-worker identity. The parser keeps structural `Cast`/`Ternary`/`Index`/`CompoundAssign` nodes; `flow` walks nested calls once, attaches branch conditions, and diagnoses only the root `Unknown` of a construct.

## Analysis boundaries

This is a bounded, dependency-free structural parser and flow analyzer, not a full PHP compiler or a replacement for PHP's grammar validator. Balanced delimiters and known malformed declarations/expressions are rejected. Unrecognized expression semantics remain unknown with `analysis_limitation`; explicitly unsupported syntax prevents replacement of that file's existing graph. Acceptance is not proof that PHP would compile the file.

Supported evidence includes namespaces and ordinary imports, named classes/interfaces/traits and their members, inheritance and trait adaptations, functions/closures/arrows, common expressions and calls, if/else flow, assignment origins and basic builtin return types. Loops and exception flow are approximate. Top-level executable statements, constants, enum-case details, attributes, promoted properties and multiple properties in one declaration do not yet have complete graph semantics. Grouped imports, anonymous classes and alternative control-flow syntax are explicitly unsupported. Dynamic dispatch and runtime framework configuration remain candidates or unknown; interprocedural execution and alias analysis are not attempted.

Target-version gates cover the listed constructs only. Selecting a PHP target does not provide full version-specific syntax validation. Resource controls include the common analysis deadline and graph-element limit, parser nesting bounds, and source-loading limits enforced by the coordinator.

## php-type-inference

Change: `openspec/changes/php-type-inference`. Требование владельца: тип из аннотации — не точный ответ, его нужно помечать.

TDD:

1. `tests/php_doc.rs` (5 тестов) сначала запускался со stub-модулем `doc.rs`, и все 5 упали. После реализации разбора PHPDoc они проходят. Тест `refine` («код важнее аннотации») написан в одном шаге с реализацией; это отклонение от TDD.
2. `tests/native_php_types.rs` (6 сценариев PLUGIN-08/09, INDEX-08) упал целиком: не было `type_source`, `annotated`, magic-свойств и flow-фильтра для vendor. Первая реализация провалила 2 сценария. Причины — ошибка в тесте (в методе два вызова `find()`) и ошибка в коде: курсор комментариев пропускал `@var` перед первой инструкцией тела. После исправления проходят 6 из 6.
3. `tests/config.rs` и `tests/project.rs` (ENV и discovery `ATLAS_DECLARATION_PATHS`) упали на компиляции из-за отсутствующего поля, затем прошли.
4. Замер на `slotor777` выявил превышение лимита анализа и +148 тыс. рёбер `available_via`. Добавлены 2 теста (`available_via` к библиотекам и `unindexed_ancestor`); оба упали, затем прошли.

Замер на `slotor777` (одноразовая PostgreSQL 16, `ATLAS_DECLARATION_PATHS=["vendor/yiisoft"]`):

| | До | После |
|---|---|---|
| Время `index` | 150 с | 56 с |
| Вершины / рёбра | 331 867 / 586 681 | 345 363 / 606 553 |
| Диагностики | 52 151 | 20 575 |
| `Definition not indexed: yii\…` | есть | 0 |
| Вызовы в проекте: resolved / annotated / candidate / unresolved / external | — | 33 978 / 10 351 / 4 170 / 10 111 / 11 964 |
| `processingGameTransaction`, рёбра `calls` | по отзыву 3 из 27 распознано | 42 resolved + 72 annotated + 16 candidate из 180; 26 unresolved, 24 external (встроенные функции PHP) |

Нераспознанными в `processingGameTransaction` остаются:

- `$ex->getMessage()`: метод встроенного `\Exception`, которого нет в индексе;
- `$redis->…`: компонент `Yii::$app->redis`, следующий change;
- цепочка `$user` из `User::find()`: `User` наследует `Da\User\Model\User` из `vendor/2amigos`. С `ATLAS_DECLARATION_PATHS=["vendor/yiisoft","vendor/2amigos"]` распознано ещё 3 вызова, время 57 с.

```sh
cargo test --locked                       # 139 passed, 5 ignored
cargo clippy --locked --all-targets -- -D warnings
```
