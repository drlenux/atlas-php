# Tasks: text-mcp-and-grep-supplement

- [x] 1.1 Proposal, design, spec delta
- [x] 2.1 Red/green: edge-aware literal word matching in grep
- [x] 2.2 Red/green: `grep_supplement` use case (coverage, mode, paging) in Service Layer
- [x] 2.3 Web and MCP call the use case; remove orchestration from `http.rs`
- [x] 3.1 Red/green: presenter for nodes, search, context, grep, outline, read, correction
- [x] 3.2 MCP returns text; HTTP/workflow tests assert text; descriptions updated
- [x] 3.3 Red/green: bare `|` (union types) stays raw in values
- [x] 4.1 Full checks; build; deploy to slotor; re-measure; update slotor atlas skill

## Evidence (2026-11-10)

TDD (red observed on the intended assertion first):
- `grep_search::tests::literal_word_checks_boundaries_only_at_identifier_edges`: red `left: []`.
- `tests/grep_supplement.rs`: with the previous behaviour moved into the service, red
  `left: [3, 5, 6, 9]` (declaration and call-site lines repeated), substring hit for
  `BonusCodLog`, supplement on later node pages; green after full-query coverage, mode
  matching and the paging rule.
- `mcp_text::tests` (9 tests): red against empty stubs, green after the presenter.
- `tests/http.rs` (11 MCP tests) and `tests/workflow.rs`: red while MCP returned JSON.

Checks: `cargo test --locked` 281 passed; lib tests stable over 3 parallel runs;
`node --test web/*.test.mjs` 19 passed; clippy and fmt clean; `core_postgres` 7 passed, 1
pre-existing failure (legacy cursor test, see `lean-search-and-class-context`).

Live slotor, local image 0.1.31 (not pushed), JSON 0.1.29 → text 0.1.31:

| Call | 0.1.29 | 0.1.31 | rg / Read |
|---|---|---|---|
| `atlas_search checkLifetime` | 1 000 B | 598 B | 137 B |
| `atlas_search getUserBonus` | 2 121 B | 1 108 B | 437 B |
| `atlas_search getUserBonus withGrep` | — | 1 185 B, supplement 0 (atlas covers all 6 rg lines) | 437 B |
| `atlas_context` method | 17 066 B | 8 018 B | — |
| `atlas_context` class | 2 381 B | 1 283 B | — |
| `atlas_outline called_by` | 359 B | 268 B | — |
| `atlas_grep checkLifetime word` | 452 B | 245 B | 137 B |
| `atlas_read` method | 367 B | 295 B | 177 B |
| tool definitions | 8 263 B | 7 960 B | — |
