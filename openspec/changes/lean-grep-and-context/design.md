# Design: lean-grep-and-context

## Grep exclusions

```
grep_search::grep_page ─► GrepPage { hits | files | counts, skipped[], excluded{reason→count}, errors[], incomplete }
```

Problem: the walk includes vendor declaration paths and repository source paths, then
each vendor file is pushed to `skipped` as `vendor_excluded` (1 196 rows ≈ 115 KB per call).
An exclusion is a scope decision, not a scan failure, so a per-file list carries no
actionable information.

Decision:
- New `GrepPage.excluded: BTreeMap<String, usize>` (omitted when empty); `vendor_excluded`
  increments it.
- `skipped[]` keeps per-file rows only for `file_too_large:*`, which drives `incomplete`.
- `incomplete` semantics unchanged. `withGrep` shares the engine, so it gets the same shape.

## Context member listings

Problem: from a method root, depth 2 walks up to the owner class (incoming `contains`)
and then lists every member of that class (`contains` 95 edges) plus inherited members
(`available_via` 138 edges). Those listings are what `atlas_outline` exists for.

Decision: `ContextRequest.members: Members` with two values.

| Value | Behavior | Used by |
|---|---|---|
| `Members::All` | current behavior | web `/api/context`, `AtlasService::context` wrapper |
| `Members::RootOnly` | outgoing `contains`/`available_via` are followed only from the root | MCP `atlas_context` |

Incoming membership edges (child → owner) are still followed, so owners stay visible.
Skipped member edges do not set `truncated` or `depth_limited`: they are omitted by
contract, documented in the tool description, not cut by a budget.

Pattern: a parameter on the existing use case (Service Layer) rather than a second
traversal; one BFS, one rule check (`Direction::follows` plus a membership filter).

## Validation

- Unit test in `grep_search` (vendor exclusion counted, not listed).
- `tests/lean_responses.rs`: method root with owner class and sibling members; RootOnly
  returns owner but not siblings, All keeps siblings.
- MCP test: `atlas_context` uses RootOnly.
- Re-measure on slotor with a locally built image.
