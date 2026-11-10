# Atlas navigation — reference

Companion to [SKILL.md](SKILL.md). Load when the main skill is not enough.

## Setup (consumer project)

1. Secrets in `.env`: `ATLAS_POSTGRES_PASSWORD`, `ATLAS_AGENT_TOKEN`, different
   `ATLAS_REVIEW_TOKEN`.
2. `docker compose up -d atlas` (or the project’s atlas service).
3. MCP client entry (HTTP), e.g.:

```json
{
  "mcpServers": {
    "atlas": {
      "type": "http",
      "url": "http://localhost:${ATLAS_HOST_PORT:-8080}/mcp",
      "headers": {
        "Authorization": "Bearer ${ATLAS_AGENT_TOKEN}"
      }
    }
  }
}
```

Export `ATLAS_AGENT_TOKEN` in the shell if the client expands `${…}` and does not
read `.env`. Keep the real `.mcp.json` gitignored.

## Extra Yii / PHP patterns

| Question | Call / note |
|---|---|
| Overrides / overridden_by | `atlas_outline(method:…, sections:["overrides","overridden_by"])` |
| Factory `const MAP = [… => Foo::class]` then `new $class` | typed via `class_string_values`; MAP keys are `read_site` |
| `Yii::createObject(X::class)` then `$x->method()` | result typed as `X`; `called_by` on `X::method` |
| `foreach ($q->batch() as $rows)` nested foreach | batch → `T[][]`, element typed as model |
| Calls inside `$fn = function () { … }` | method outline lists nested-closure call sites |
| `Event::on` / `$obj->on` / `trigger` | `kinds:["event"]`; literals ↔ `EVENT_*`; `off` → `unsubscribes` |
| Interface method with no callers | `called_by: none` is definite for direct calls; still check `overridden_by` |

## Outline sections (common)

Class: `members`, `parents`, `children`, `uses`, …  
Method: `calls`, `called_by`, `overrides`, `overridden_by`, …  
Empty requested section prints `<section>: none`.

## atlas_batch

Only read-only tools: `atlas_search`, `atlas_context`, `atlas_outline`, `atlas_grep`,
`atlas_read`. One shared `# index` line; each call is `## [n] <tool>` (errors do
not stop siblings).

## SQL edges and DDL files

- `sql_reads` / `sql_writes`: from `sql_site` → `table:schema.name` (default schema
  `public`). CTE names are not tables.
- DDL in `.sql` when `pgsql` is in `index.languages`: `schema`, `table`, `column`,
  `view`, `function`, `index`, plus `contains` / `references` / `indexes`.
- AR attribute writes remain `write_site`, not `sql_site`.

## Result quirks

- `atlas_context` trees: `[area]` prefix on lines; `omitted_members=N` → use outline
  for the root’s members.
- `withGrep:true` on search: `# grep supplement` for source hits atlas did not return;
  continue with `grepCursor`.
- `excluded.binary`, path excludes, and vendor rules come from setup / built-ins.
