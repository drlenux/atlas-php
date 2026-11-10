# Design: text-mcp-and-grep-supplement

## Components

```
MCP call_tool ─► AtlasService use cases ─► domain structs ─► mcp_text (presenter) ─► text content
Web /api/*    ─► same use cases ─────────► domain structs ─► serde JSON (unchanged)
```

- `src/mcp_text.rs`: presenter. Pure functions from domain structs to text; no I/O, no
  repository access. Pattern: Presenter at the adapter boundary, so use cases stay
  format-agnostic and the web keeps JSON.
- `AtlasService::grep_supplement`: use case that was inside `http::search_payload`. Both
  MCP and web call it (Service Layer rule: no orchestration in handlers).
- `AtlasService::read_source` returns a `SourceSlice` struct instead of `Value`, so the
  presenter and the web serialize the same typed result.

## Text format

First line of every successful result:

```
# index state=completed indexed_at=1794291574
# index state=running indexed_at=1794291574 changes_pending=true pending=a.php,b.php
```

Value rendering (`key=value`): strings raw when they contain no whitespace or `"` and do
not start with `[`/`{`, otherwise JSON-quoted; numbers raw; `true` booleans as a bare key,
`false` omitted; `null` and empty values omitted; arrays/objects compact JSON.

Node line:

```
<path>:<start>[-<end>]: <kind> <name> [<resolution>] if(<condition>)… <key=value>… [qn=<qualified>] | id=<id> [rev=<n>] [trust=<t>]
```

- `name` whitespace runs collapse to one space; `metadata.expression` equal to the name is
  not repeated.
- `resolution` and `conditions` get the dedicated forms above; other metadata keys follow
  sorted.
- `rev` when non-zero; `trust` when not 1 (analyzer).

`atlas_search`:

```
# index …
# search nodes=2 complete                    (or next_cursor=<cursor>)
<node lines>
# grep supplement hits=1 complete files=3484 excluded vendor_excluded=1196
<path>:<line>:<col>: <text>
```

`atlas_context`: nodes numbered `[n]` (root is `[1]`), edges reference them.

```
# context root=[1] nodes=18 edges=26 truncated=false depth_limited=true omitted_members=136
[1] <node line>
# edges
[1] -contains-> [4] <path>:<line> [resolved] <key=value>… | id=<edge id> rev=<n>
# diagnostics
<path>:<line>: <code> <message>
```

An endpoint outside the listed nodes is printed as its id; a missing `to` as `?`.

`atlas_grep` (ripgrep conventions): `path:line:col: text` for matches, `path-line- text` for
context lines, `path` per line for `files_with_matches`, `path:count` for `count`; header
`# grep files=… hits=… complete|incomplete|next_cursor=… excluded …`; `# skipped <path>
<reason>` and `# error <path>: <message>` lines. Commented hits end with ` [commented]`.

`atlas_outline`: header line `<kind> <name> <path>:<line> | id=<id>`, scalar fields as
`owner: …`, list fields as a section name followed by two-space indented lines.

`atlas_read`: `# <path>:<start>-<end>[ truncated]`, then `<line>: <text>`.

`atlas_correct`: `# correction` then `key=value` per field.

Errors keep the existing `isError` text content.

## Grep supplement

Problem: see proposal. Decision:

| Search mode | Grep needle | Match |
|---|---|---|
| `text` | query | literal, identifier boundaries at identifier edges |
| `substring` | query | literal substring |
| `symbol` | short name (`grep_query_for_mode`) | identifier, symbol-form ranking |

Coverage: `search_nodes` with the same text, mode and kinds, no cursor, up to
`SUPPLEMENT_COVERAGE_LIMIT` (10 000) nodes. Occurrence nodes cover their span lines;
other nodes cover their start line (declaration line). Class/method spans still never hide
inner lines.

Paging: the supplement is computed when the node cursor is empty or `grepCursor` is given;
otherwise it is omitted so node paging does not repeat it.

## Edge-aware literal word matching

`grep_search::literal_score` with `word` requires a boundary only where the needle starts or
ends with an identifier character, matching `repository::contains_identifier`. Regex `word`
keeps the `\b` wrap.

## Validation

- Unit tests for the presenter (node line, edge refs, grep lines, outline, read).
- Service tests for the supplement (declaration line hidden, coverage beyond the page,
  mode-consistent matching, no supplement on later node pages).
- `tests/http.rs` / `tests/workflow.rs`: MCP results are text; web stays JSON.
- Live slotor re-measurement.
