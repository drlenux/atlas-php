# Tasks: agent-efficiency

- [x] 1.1 Proposal, design, spec delta
- [x] 2.1 Red/green: glob module (classes, negation, alternatives, `**`)
- [x] 2.2 Red/green: grep scans every text file; binary excluded; lossy decoding; own size limit
- [x] 2.3 Red/green: `path_glob` list with `!` exclusions
- [x] 3.1 Red/green: `atlas_batch` (read-only tools only, no nesting)
- [x] 4.1 Red/green: outline prints requested empty sections as `none`
- [x] 4.2 Red/green: interface method `overrides` edges (native PHP)
- [x] 5.1 Red/green: search excludes test paths by default (memory + PostgreSQL); withGrep coverage keeps them
- [x] 5.2 Red/green: text mode ignores string literals (memory + PostgreSQL)
- [x] 5.3 Red/green: callers tree use case and presenter; MCP direction in; outline `called_by` test exclusion
- [x] 5.4 Red/green: class root usages grouped by member; class context defaults to usages
- [x] 5.5 Red/green: graph edges omit metadata repeated from the source node
- [x] 5.6 Red/green: full expression instead of truncated name; `receiver_root_class` only when different;
      two innermost conditions in the tree; binary before size check; no `complete` with `incomplete`
- [x] 6.1 Full checks; build; deploy to slotor; re-measure
- [x] 6.2 Update slotor atlas-navigation skill (and allowlist `atlas_batch`)
- [x] 6.3 Benchmark on two specs (`docs/benchmark-agent.md`)

## Evidence (2026-11-10)

TDD: every item above had its test fail on the intended assertion before the change. Cases
worth noting:
- the batch read-only test first passed for the wrong reason (stale revision); it was fixed to use
  the real revision, then failed until `BATCH_TOOLS` was added;
- `excluded_test_paths_still_cover_their_lines` was added after the coverage change and shown to
  fail when the change is reverted;
- three older tests asserted the replaced behaviour and were updated deliberately: text search inside
  string literals (`agent_navigation`, `core_postgres`), `direction:in` as a graph (`http`).

Checks: `cargo test --locked` 313 passed; lib tests stable over 3 parallel runs; web 19 passed;
clippy and fmt clean; `core_postgres` 8 passed, 1 pre-existing failure (legacy cursor test).

Live slotor, local image 0.1.33 (not pushed):

| Call | 0.1.31 | 0.1.33 |
|---|---|---|
| `atlas_context allows direction:in depth:3` | 11 559 B, callers of `createFreeSpin` missing | 2 510 B, 3 call levels, 14 callers, `excluded.tests=5` |
| `atlas_context` class `FreespinQuotaPolicy` | 1 node | usage tree by member (3 members, 1 unused) |
| `atlas_outline updateLimits called_by,overridden_by` | header only | `overridden_by` 3 implementations, `called_by: none` |
| `atlas_search createFreeSpin kinds:[call_site]` | 22 285 B, 30 rows (19 noise) | 15 194 B, 22 rows, tests excluded, no log strings |
| `atlas_grep aggregatorua path_glob:*.sh` | `scanned=0` | `scanned=11` |
| `atlas_grep` whole tree | php/js/html/vue only | 5 083 text files, `excluded.binary=842`, 1.7 s |
| `atlas_batch` outline + grep + read | 3 calls | 1 call, 2 823 B |
