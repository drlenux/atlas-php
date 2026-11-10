# Proposal: lean-search-and-class-context

## Why

After `lean-grep-and-context` (image 0.1.27) live slotor measurements still show:

| Call | Payload | Problem |
|---|---|---|
| `atlas_search getUserBonus` (text) | 10 284 B, more pages | substring matches `$targetUserBonus`, `getUserBonusCod`, `getUserBonusTask`; rg -w finds 6 lines (437 B) |
| `atlas_search` nodes | ~25% of bytes | `qualified_name` repeats `id` without the `<kind>:` prefix |
| `atlas_context` class root | 256 717 B (200 nodes, truncated) | lists every member and their usages; `atlas_outline` lists the same members in 6 747 B |

## What changes

- `atlas_search` mode `text` matches whole identifiers at the query edges (like `grep -w`).
  The previous behaviour stays available as mode `substring`. The reviewer web search keeps
  substring as its default.
- Summary projections drop `qualified_name` when it equals `id` without the `<kind>:` prefix.
- MCP `atlas_context` never lists declaration members (method, property, constant, …).
  A root still returns its own occurrence sites, branches and local variables. The response
  reports `omitted_members` for the root and the tool description points to `atlas_outline`.
  Supersedes the "Class context lists its members" scenario of `lean-grep-and-context`.

## Out of scope

- Compact line format for context edges.
- Postgres trigram indexes for search.
