> Historical verification of the pre-v0.4 implementation. PHP subprocess / Git CLI references below are obsolete. Current native-plugin evidence is recorded in `openspec/changes/add-code-atlas/verification.md` and the native testing reports.

# Core validation

Architecture and contracts were committed before implementation in `openspec/changes/add-code-atlas/design.md`.

## TDD evidence

1. Added `tests/core_behavior.rs` before the domain, service and repository modules. The first `cargo test --offline --test core_behavior` failed with unresolved `code_atlas::domain`, `repository`, and `service`: the specified API did not exist.
2. Implemented the domain, transaction boundary, memory adapter and service. Eight initial behavior tests passed.
3. Added the retarget/deletion regression before its fix. The test failed with `left: 0, right: 1`: indexing incorrectly deleted a surviving updated edge when its former target disappeared. Changed deletion reconciliation to inspect current edges; the regression passed.
4. Added ambiguity coverage and ran the complete core behavior suite: 10 passed. Rustfmt was applied and the suite passed again.
5. Added `source_paths_are_complete_and_deduplicated_without_search_limit` first; the test failed because `AtlasService::source_paths` did not exist. Implemented a complete path query, including a PostgreSQL DISTINCT query, and the suite passed: 11 tests.
6. Integration review with the PHP agent identified structural IDs inside metadata (`scope`, origin IDs/definitions/candidates and edge candidates). Added a rename regression: it failed with `new` instead of preserved ID `old`. Implemented remapping only for the documented reference fields, preserving source text; all 12 behavior tests passed.
7. Added two edge identity regressions before fixes. An edited call-site ID deleted a protected edge; an ambiguous rename of a trust-1 endpoint cascaded deletion of its protected edge. Both failed. Reconciliation now requires unique edge fingerprint matching and retains ambiguous protected edges and endpoint identities without promoting endpoint trust. All 14 core behavior tests and both real-plugin integration tests passed afterward.
8. Added rename-followed-by-body-edit regression for both trust-1 and protected records, including an unordered scoped variable. It failed because the retained immutable ID no longer matched the analyzer's renamed ID and the body fingerprint had changed. Added unique current semantic-identity matching before structural fallback, resolving scopes until stable. All 15 core tests and the two real-plugin integrations passed.
9. Added old-FQN reintroduction regression before its fix: after A was renamed to B with trust 3, a new declaration A stole the original ID and its trust. Raw-ID matches now require compatible current semantic identity, and new declarations whose analyzer key is already reserved receive a deterministic checked suffix. Edges and known metadata references use the selected persistent IDs. The reordered repeat-snapshot case preserves both records. All 16 core tests and both real-plugin integrations passed.

Coverage: immediate agent corrections, lower-trust reindex protection with position refresh, approval, stale revisions, chained decline restoration, immutable-field and target validation, structural rename matching, ambiguity preservation, confirmed deletion, transaction rollback, context bounds, analytics deduplication, half-open periods, and retained labels after deletion.

## Running

```sh
cargo test --test core_behavior
cargo test --test core_postgres -- --ignored --test-threads=1
cargo test --test core_analyzer -- --ignored
```

The PostgreSQL target requires `ATLAS_TEST_DATABASE_URL` pointing to a disposable database. Its three explicit integration tests cover actual foreign-key failure/rollback, independent connections and revision checks, correction/review/analytics persistence, and SQL parameterization including quotes and Unicode. They are ignored in the default test run; compiling them is not proof of SQL correctness.

## PostgreSQL environment

PostgreSQL 17.11 and libpq are installed locally. Initial temporary-cluster startup failed during `initdb` with `shmget: Operation not permitted`; the sandbox does not permit shared-memory allocation. Retrying with supported `shared_memory_type=mmap` and `dynamic_shared_memory_type=mmap` settings failed at the same mandatory small shared-memory segment on macOS. No test server started, and no production database was accessed.

Verified locally: 16 behavior tests passed; the 3 PostgreSQL integration tests compiled and were explicitly ignored because a disposable PostgreSQL server was unavailable. The native adapter compiles and links against the installed libpq. Real SQL execution remains an environment-dependent verification gap.

Static adapter review used PostgreSQL 17's [command execution contract](https://www.postgresql.org/docs/17/libpq-exec.html), [thread ownership rules](https://www.postgresql.org/docs/17/libpq-threading.html), and [CREATE TABLE syntax](https://www.postgresql.org/docs/17/sql-createtable.html). Parameterized dynamic values and serialized connection ownership follow these contracts; this review is not runtime verification.

## Actual analyzer integration

The 2 tests in `core_analyzer` were executed successfully against the actual PHP plugin and Composer-installed AST parser from the analyzer worktree. They cover the combined domain/Yii/vendor fixture accepted by the service, interface candidates and separate returns in context, and an actual parsed class rename preserving method trust, positions, variable scope references and persistent IDs. Optional `ATLAS_TEST_PLUGIN` and `ATLAS_TEST_FIXTURES` paths support worktree testing; by default they resolve within the checkout. These tests are explicit because they require PHP and installed plugin dependencies.
