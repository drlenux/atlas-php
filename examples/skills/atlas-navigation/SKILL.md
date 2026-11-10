---
name: atlas-navigation
description: PHP/Yii2 code graph via Code Atlas MCP (atlas_search, atlas_outline, atlas_context, atlas_grep, atlas_read, atlas_batch). Use for callers/callees, field writes with branch conditions, raw SQL tables (createCommand/findBySql), class structure, and when Grep is still required (views, JS, translations, Query Builder without SQL strings).
---

# Code graph navigation (Code Atlas)

Code Atlas indexes PHP (Yii2 plugin) into a graph: declarations plus occurrences
(`call_site`, `read_site`, `write_site`, `return_site`, `branch`, `sql_site`) with
branch `conditions`. MCP: `http://localhost:${ATLAS_HOST_PORT:-8080}/mcp` with
`Authorization: Bearer ${ATLAS_AGENT_TOKEN}`. Restart the agent after MCP config
changes. Rare cases and extra Yii patterns: [reference.md](reference.md).

## Strategy (fewest calls)

1. **Narrow search first** — `atlas_search` with `kinds` + `mode:"symbol"` when you
   know the name; `atlas_grep` / `withGrep:true` for text, configs, comments.
2. **Callers** — direct: `atlas_outline(…, sections:["called_by"])`; up the tree:
   `atlas_context(entity_id, direction:"in", depth:3)`.
3. **Callees** — `atlas_outline(…, sections:["calls"])`, not `context direction:"out"`.
4. **Read narrowly** — `atlas_read(path, start_line, end_line)` or
   `around_line`+`context`; `entity_id` can dump 150+ lines.
5. **Batch** — `atlas_batch` up to 10 independent read-only calls.

Tests (`tests/`, `*Test.php`, `*Cest.php`) are excluded unless `include_tests:true`
(`excluded.tests=N`). Scopes live in `.atlas/setup.yaml`; `.env` is never read.

- **Areas** (from setup, e.g. `frontend`/`backend`/`console`/`common`): lines show
  `[area]`; `area:"backend"` (or a list) filters search/grep/outline callers/context.
  Skip `area` when the answer may cross apps. Namespace-less IDs:
  `class:<area>:<Name>`.
- **`undeclared`**: receiver fully known, member missing → runtime failure. Search
  `kinds:["call_site"]`.
- **`[unresolved: name only]`** / `possible.name_only=N`: untyped same-name calls —
  verify by reading; not proven callers.

## Which tool for which question

| Question | Call |
|---|---|
| Direct callers / callees | `atlas_outline(…, sections:["called_by"])` / `["calls"]` |
| Callers up to entry points | `atlas_context(id, direction:"in", depth:3)` |
| Who uses class C from outside | `atlas_context(class:…)` |
| Interface `I::m` callers + impls | `outline(method:I::m, sections:["called_by","overridden_by"])` |
| Queue job dispatchers | `outline(method:…::execute, sections:["called_by"])` — `queue <name>` |
| Where field F is written (+ branch) | `search(query:"Owner::$F" or FQCN, kinds:["write_site"], mode:"symbol")` |
| Where F is read | `kinds:["read_site"]` (prefer over mixing via context) |
| Who runs raw SQL on table T | see **Raw SQL** below |
| Call sites of a member | `search(query:"name", kinds:["call_site"], mode:"symbol")` |
| Class by short name | `search(query:"Name", kinds:["class"], mode:"symbol")` — always set `kinds` |
| Class / method structure | `atlas_outline(class:…)` / `outline(method:…)` |
| Text / config / comments | `atlas_grep`; graph key → `kinds:["config_key"]` |
| Route / URL sites | `search(…, kinds:["route","url_site"])` |
| Read source | `atlas_read` (path slice or `around_line`) |

Build IDs when obvious: `class:app\models\Order`, `method:…::pay`,
`property:…::$amount`, `table:public.wallet`. **Do not invent** `sql_site:…` or
`call_site:…` fingerprints — copy `id=` from results.

## Raw SQL

Indexed from string args to `createCommand` / `findBySql` / `prepare` (literal or
`$sql = "…"` then that variable). `.sql`/`.pgsql` files need
`index.languages: […, pgsql]`.

1. `atlas_search(query:"wallet", kinds:["table"], mode:"symbol")` → `table:public.wallet`
2. Find sites: `atlas_search(query:"wallet", kinds:["sql_site"], mode:"substring")`
   or `atlas_grep(query:"UPDATE wallet")` / `withGrep` on the table name
3. Open the method (`atlas_read` / outline); edges `sql_reads` / `sql_writes` link
   `sql_site` → table

Not SQL graph: Query Builder / `batchInsert` / AR `save`/`updateAttributes` (use
`write_site` / grep), concatenated SQL without a recoverable literal.

## Don’t

- `atlas_search` without `kinds`, or bare field name `F` across every class
- `atlas_context(direction:"out")` for callees → use outline `calls`
- Treat empty `called_by` as unused → confirm with `withGrep` / `atlas_grep`
- `atlas_read(entity_id)` on huge methods when a line slice suffices
- `atlas_correct` unless the user asks (shared index, trust level 2)

## Reading results

Grep-style text, not JSON. Header `# index state=… indexed_at=…` (+ `pending=`).
Lines: `path:line: kind name [resolution] if(cond)… key=value | id=… rev=…`.

- `resolved` / `annotated` (verify PHPDoc) / `candidate` / `unresolved`/`external`
- Outline tags: `magic`, `annotation`, `framework`
- `conditions`: outermost branch first; `!(…)` = else
- Search/grep: follow `next_cursor` / `grepCursor` until `complete`; `incomplete` =
  skipped files, not “no hits”
- `mode:"text"` ≈ `grep -w`, skips string literals; `substring` partial; `symbol` exact

## Freshness

If `state` ≠ `completed` or your file is in `pending=`, use Read/Grep for that file
until the watcher finishes (debounce from setup, then incremental rebuild).

## Use Grep / atlas_grep (atlas is blind)

- Which action `render()`s a view/partial (view **bodies** are indexed; links are not)
- Translations, comments/docblocks, most JS logic
- Config text outside indexed `config_key`s; Query Builder / non-literal SQL
- Type gaps: missing `called_by` is not proof of unused — grep before renames/deletes
- Rename across refs: atlas is read-only
