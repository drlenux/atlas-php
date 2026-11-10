# Tasks: lean-search-and-class-context

- [x] 1.1 Proposal, design, spec delta
- [x] 2.1 Red/green: memory text search matches whole identifiers; `substring` mode
- [x] 2.2 Red/green: PostgreSQL search follows the same rules
- [x] 2.3 Web search defaults to substring; MCP schema lists `substring`
- [x] 2.4 Red/green: variable scope (qualified name) is not a text match (memory + PostgreSQL)
- [x] 2.5 Refactor: strpos prefilter before regex in PostgreSQL
- [x] 3.1 Red/green: summary drops kind-prefixed duplicate `qualified_name`
- [x] 4.1 Red/green: MCP context omits declaration members with `omitted_members`
- [x] 4.2 Red/green: MCP context follows `inherits`/`implements`/`uses_trait` only from the root
- [x] 5.1 Full checks; build image; deploy to slotor; re-measure

## Evidence (2026-11-10)

TDD (each red observed on the intended assertion before the change):
- `tests/search_modes.rs::text_mode_matches_whole_identifiers_only`: red returned
  `getUserBonusCod` and `$targetUserBonus`; later red returned the scoped `$this` variable.
- `tests/core_postgres.rs::postgres_text_search_matches_whole_identifiers_and_substring_keeps_partial`:
  same two reds on PostgreSQL (`atlas-pg-test`), green after the `q` CTE and the variable rule.
- `tests/http.rs::reviewer_search_defaults_to_substring_while_agent_text_needs_whole_identifier`.
- `tests/lean_responses.rs::summary_drops_qualified_name_derivable_from_kind_prefixed_id`:
  red `left: "Wallet::refund"`.
- `tests/context_members.rs::omit_class_context_counts_members_instead_of_listing_them` and
  `focused_class_context_returns_parent_without_its_other_children` (sibling, then trait user).

Checks: `cargo test --locked` 267 passed; `node --test web/*.test.mjs` 19 passed; clippy and
fmt clean; `core_postgres` 7 passed, 1 failed. The failure is
`postgres_search_filters_kinds_and_walks_keyset_cursor` and also fails before this change: it passes a
legacy bare-id cursor while ranked cursors are `score|id`.
`openspec validate --all` fails only on `batch-foreach-and-closures` and `yii2-events` (no deltas,
pre-existing).

Live slotor, local image 0.1.29 (not pushed):

| Call | 0.1.27 | 0.1.29 |
|---|---|---|
| `atlas_search checkLifetime` | 1 478 B (3 nodes) | 1 000 B (2 nodes) |
| `atlas_search getUserBonus` | 10 284 B (20 nodes, more pages) | 2 121 B (6 methods, last page) |
| `atlas_context` method | 18 845 B | 17 066 B |
| `atlas_context` class | 256 717 B (200 nodes, truncated) | 2 381 B (4 nodes, `omitted_members` 136) |

Search latency about 0.25 s end to end. A full scan of 389k nodes costs 100–140 ms in PostgreSQL
with or without the regex.
