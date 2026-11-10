# Design: atlas-source-search

## Goals

- One MCP replaces external grep+Read for PHP project text, while the graph remains the
  place for typed links (`resolution`: resolved / annotated / candidate / unresolved).
- Honest completeness: empty hits ≠ failed/incomplete scan.
- Economical: grep returns matches only; bodies via `atlas_read`; callers via outline sections.

## Architecture

```
MCP atlas_grep  → GrepArgs → AtlasService::grep_source → project::SourceWalk + grep_search::scan
MCP atlas_read  → ReadArgs → AtlasService::read_source → filesystem (live) [+ node.span for entity]
MCP atlas_search withGrep → same scan engine; suppress only occurrence-site lines already in page
```

Text is always read from the live tree under `ATLAS_PROJECT_ROOT`. Graph freshness
(`index`) is attached so the agent knows when typed links may lag the text.

## atlas_grep

| Arg | Default | Notes |
|---|---|---|
| `query` | required | non-empty; length 1+ allowed |
| `regex` | false | Rust regex crate |
| `case_sensitive` | true | false → case-insensitive |
| `word` | false | word boundaries (literal or `\b` wrap for regex) |
| `path_prefix` | — | relative prefix filter |
| `path_glob` | — | `*` / `?` (same wildcard as ATLAS_EXCLUDE) |
| `include_vendor` | false | when false, vendor paths listed in `skipped` |
| `output` | `content` | `content` \| `files_with_matches` \| `count` |
| `context` | 0 | lines before/after match (0..5), content mode |
| `limit` | 50 | 1..200 hits / files / count rows |
| `cursor` | — | `path:line` exclusive resume |

Response always includes: `scope` (roots, file_count, include_vendor), `skipped[]`
(`path`,`reason`), `errors[]` (`path`,`message`), `truncated`, `next_cursor`,
`incomplete` (true if errors or size skips prevented a full scan of intended files),
`index` freshness. Text is live; `incomplete=false` and empty hits means no match.

Snippet: window centered on the match (not prefix-200). Column optional.

## atlas_read

| Arg | Notes |
|---|---|
| `path` + `start_line`/`end_line` | live file slice (max 400 lines) |
| `entity_id` | use node span; read live file |
| `around_line` + `context` | convenience around one line |

## withGrep fix

Covered lines = only kinds `call_site|write_site|read_site|return_site|url_site|branch`
at their `start_line..=end_line`. Class/method spans never suppress text hits.

## Patterns

- Service Layer owns use cases; HTTP only parses/bounds.
- Shared `grep_search::scan` for `atlas_grep` and `withGrep`.

## Risks

- Regex ReDoS: bound input length (≤500) and per-file timeout via line cap.
- Large trees: skip >`ATLAS_MAX_FILE_BYTES`, report in `skipped`; page with cursor.
