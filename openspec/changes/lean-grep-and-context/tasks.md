# Tasks: lean-grep-and-context

- [x] 1.1 Proposal, design, spec delta
- [x] 2.1 Red: vendor exclusion counted in `excluded`, not listed in `skipped`
- [x] 2.2 Green: `GrepPage.excluded`; tool description
- [x] 3.1 Red: RootOnly context omits sibling members of the owner, keeps owner
- [x] 3.2 Green: `Members` on `ContextRequest`; MCP uses RootOnly, web All
- [x] 4.1 Fix flaky `tempfile_dir` collision in grep unit tests
- [x] 4.2 `cargo test`, clippy, fmt, `openspec validate`
- [x] 4.3 Build image, deploy to slotor, re-measure

## Evidence (2026-11-10)

TDD:
- `grep_search::tests::vendor_exclusion_is_counted_not_listed`: red `left: None, right: Some(3)`
  with an empty `excluded` field; green after counting.
- `tests/context_members.rs::root_only_method_context_returns_owner_without_sibling_members`:
  red with `Members` present but ignored; green after `lists_unrequested_members`.
- `tests/http.rs::agent_context_omits_owner_members_while_reviewer_keeps_them`: red with
  MCP forced to `Members::All`, green with `RootOnly`.

Checks: `cargo test --locked` 261 passed, 0 failed; lib tests stable over 5 parallel runs;
`cargo clippy --locked --all-targets -- -D warnings` clean; `cargo fmt --check` clean;
`openspec validate lean-grep-and-context --strict` valid.

Live slotor (389k nodes), local images, not pushed:

| Call | 0.1.26 | 0.1.27 |
|---|---|---|
| `atlas_grep checkLifetime word` | 110 919 B | 452 B |
| `atlas_grep getUserBonus word` | 111 435 B | 968 B |
| `atlas_context` method default | 136 705 B (110 nodes, 263 edges) | 18 845 B (19 nodes, 29 edges) |

Known gap: `atlas_context` for a class root is still 256 717 B (200 nodes, truncated);
member listing of the root is by contract, `atlas_outline` is the cheap path.
