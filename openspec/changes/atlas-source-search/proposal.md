# Proposal: atlas-source-search

## Why

Piastrix navigation showed Atlas is valuable for typed links, but agents still need
external `rg`/`Read` for text, configs, comments and code bodies. `withGrep` is not a
grep replacement: short queries are rejected, vendor/large files fail silently, class
spans hide matches, and snippets can omit the match.

## What changes

- New MCP tools `atlas_grep` and `atlas_read` so search + graph + source stay on one server.
- Full-project text search: literal/regex, case, word boundary, path filters, output modes
  (content / files_with_matches / count), pagination, skipped/errors/truncation reporting.
- Live file reads (line range or entity body) with index freshness.
- Fix `withGrep` dedupe: only occurrence-site lines, match-centered snippets, no silent skips.

## Out of scope

- Changing PHP resolution semantics (resolved / annotated / candidate / unresolved stay).
- Replacing the graph tools; grep complements them.
- Semantic/embedding search.
