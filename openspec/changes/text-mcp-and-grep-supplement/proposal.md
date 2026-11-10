# Proposal: text-mcp-and-grep-supplement

## Why

- MCP tool results are JSON. Keys, quotes and escaped backslashes in PHP namespaces repeat on
  every node and edge; an `atlas_search` hit costs about 7× the bytes of the equivalent
  `rg -n` line. The owner asked for grep-like text that still carries all atlas data.
- `withGrep` was meant as "run a real grep over the code and append only what atlas did not
  return". Live slotor shows it does not do that:
  - the declaration line of a method already returned by atlas comes back as a grep hit;
  - in `text` mode grep matches substrings (`BonusCod` → `BonusCodLogSearch`), while atlas
    `text` now matches whole identifiers;
  - duplicates are removed only against the current atlas page, and every node page repeats
    the same grep hits.

## What changes

- Every MCP tool returns plain text: grep-style `path:line:` lines with kind, name,
  resolution, conditions, remaining metadata as `key=value`, and `id`/`rev` for follow-up
  calls. Index freshness is the first line. The reviewer web API stays JSON.
- `withGrep` becomes a supplement use case in the Service Layer, shared by MCP and web:
  - grep uses the same match rule as the search mode (`text` → whole identifier,
    `substring` → substring, `symbol` → short name as identifier);
  - lines covered by any atlas node matching the same query and kinds are removed
    (occurrence spans, declaration start lines), not only those on the current page;
  - the supplement comes with the first node page or when `grepCursor` is passed.
- Literal `word` matching in `atlas_grep` checks boundaries only at identifier edges of the
  query, the same rule as atlas `text` search.

## Out of scope

- Web interface changes.
- New atlas data; the text carries what the JSON carried.
