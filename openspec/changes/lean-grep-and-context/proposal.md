# Proposal: lean-grep-and-context

## Why

Live measurement on slotor (image 0.1.26, 389k nodes) for the task "who calls
`BonusCod::checkLifetime`":

| Call | Payload | Useful part |
|---|---|---|
| `rg -n '\bcheckLifetime\b'` | 137 B | 2 lines |
| `atlas_grep word=true` | 110 919 B | hits 259 B; `skipped` lists 1 196 vendor files as `vendor_excluded` |
| `atlas_context` default | 136 705 B | 110 nodes, 263 edges; 233 edges are member listings (`contains`/`available_via`) of the owner class |

Agents spend 1.6–2× the tokens of a grep-only session; these two defaults dominate.

## What changes

- `atlas_grep` (and `withGrep`): intentionally excluded files are reported as counts per
  reason (`excluded: {"vendor_excluded": N}`), not one `skipped` row per file. `skipped`
  keeps only files that make the scan incomplete (size limit).
- MCP `atlas_context`: member listings (`contains`/`available_via` outgoing edges) are
  expanded only for the root entity. Owners are still returned; their members are not.
  The reviewer web `/api/context` keeps the full expansion.

## Out of scope

- `atlas_search` default mode and `qualified_name` duplication (separate change).
- Compact line formats for context edges.
