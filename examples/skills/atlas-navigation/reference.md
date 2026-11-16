# Atlas navigation — reference

Companion to [SKILL.md](SKILL.md). Load when the main skill is not enough.

## Setup (consumer project)

1. Secrets in `.env`: `ATLAS_POSTGRES_PASSWORD`, `ATLAS_AGENT_TOKEN`, different
   `ATLAS_REVIEW_TOKEN`.
2. `docker compose up -d atlas-db atlas` (MCP/app + stock PostgreSQL).
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

Project scopes and MCP output filters live in `.atlas/setup.yaml`
(`mcp.omit_resolutions`, optional `mcp.response_script`, optional `logging`).

## Extra Yii / PHP patterns

| Question | Call / note |
|---|---|
| Overrides / overridden_by | `atlas_outline(method:…, sections:["overrides","overridden_by"])` |
| Factory `const MAP = [… => Foo::class]` then `new $class` | typed via `class_string_values`; MAP keys are `read_site` |
| `Yii::createObject(X::class)` then `$x->method()` | result typed as `X`; `called_by` on `X::method` |
| `foreach ($q->batch() as $rows)` nested foreach | batch → `T[][]`, element typed as model |
| Calls inside `$fn = function () { … }` | method outline lists nested-closure call sites |
| `Event::on` / `$obj->on` / `trigger` | `kinds:["event"]`; literals ↔ `EVENT_*`; `off` → `unsubscribes` |
| Interface method with no callers | `called_by: none [proof=complete]` = no resolved and no name-only residue; `proof=weak` means confirm with `atlas_grep`. Still check `overridden_by` |

## Outline sections (common)

Class: `members` (methods + Yii2 `view:…`), `parents`, `children`, `uses`, …  
Method: `calls`, `called_by`, `overrides`, `overridden_by`, …  
`table` / `column` / `property` / `sql_site`: `called_by` = access sites (+ `← maps_*`);
`members`/`calls` = outgoing `sql_reads` / `sql_writes` / `maps_*` / `contains`.  
View: `owner`, `called_by` (`renders`), `members` (injected params).  
Empty requested section prints `<section>: none`.

## atlas_batch

Only read-only tools: `atlas_search`, `atlas_context`, `atlas_outline`, `atlas_grep`,
`atlas_read`. One shared `# index` line; each call is `## [n] <tool>` (errors do
not stop siblings).

## SQL edges and DDL files

- `sql_reads` / `sql_writes`: from `sql_site` → `table:schema.name`,
  `column:schema.table.field`, and (via `maps_column`) `property:Model::$attr`.
  SELECT list + WHERE/JOIN ON/HAVING = read; UPDATE SET / INSERT cols = write.
  Default schema `public`. CTE names are not tables.
- `maps_table`: ActiveRecord class → `table:…` from `tableName()` (or camel2id).
- `maps_column`: `property:Model::$attr` → `column:…` from `rules()` /
  `behaviors()` / `attributeLabels()` / `scenarios()` / DDL columns on the table.
- Also attribute R/W: `->attributes['f']`, `$model['f']` (known attr), `load` /
  `loadMultiple`, `save`/`insert`/`upsert` (writes) / `delete` (reads), QB
  `where`/`select`/`joinWith`/`with`/`updateAll`/`batchInsert`. Fluent
  `ActiveQuery<T>` keeps `T` across `where`/`andWhere`/….
- SQL string args: literals, `$sql = "…"`, or `.` concat of **only** string
  literals (dynamic pieces → no `sql_site`).
- DDL in `.sql` when `pgsql` is in `index.languages`: `schema`, `table`, `column`,
  `view`, `function`, `index`, plus `contains` / `references` / `indexes`.
- AR attribute writes remain `write_site`, not `sql_site`.
- Yii2 views: controller `contains` the `view:…` node; action `renders` it;
  top-level template body → `function:file:{path}`.

## declaration_paths and ActiveRecord parents

Default `index.declaration_paths` is often only `vendor/yiisoft`. App models that
`extend` a library base outside that list (e.g. `Da\User\Model\User` under
`vendor/2amigos`) still get `X::find()` → `ActiveQuery<X>` via an unindexed-parent
fallback, but other inherited members stay invisible. Prefer adding the real vendor
prefix to `declaration_paths` when you need the full base class.

## Switch / case

Each `case` / `default` is a fresh entry: a `return` or `break` in one arm does
**not** mark later arms `unreachable_after_exit` (provider callbacks with
`case 'credit': return …; case 'bet': …`).

## Result quirks

- `atlas_context` trees: `[area]` prefix on lines; `omitted_members=N` → use outline
  for the root’s members.
- `withGrep:true` on search: `# grep supplement` for source hits atlas did not return;
  continue with `grepCursor`.
- `excluded.binary`, path excludes, and vendor rules come from setup / built-ins.
- `[unresolved: name only]` callers are hidden by default (`mcp.omit_resolutions`, default
  `[name_only]`); a hidden line is counted as `# omitted name_only=N`. Set `[]` to see them.
  `possible.name_only=N` can still appear with `called_by: none`.
- Class outline folds magic members (PHPDoc `@property`, AR columns) into one line
  `magic (N): $a: int, …`; `sections:["members"]` lists each on its own line.

## Формат called_by

`<caller> L<line> guards:[if(cond) return, …]: <expression> [resolution]`

- `<expression>` — вызов с аргументами, как в исходнике (многострочный сворачивается в
  одну строку).
- `guards` — до 3 ближайших early-exit проверок вызывающего **перед** вызовом; нет
  поля = гардов нет. Остальное тело — через `atlas_read`.
- `conditions` (`if(…)`) — условия, внутри которых стоит сам вызов.
- В дереве `atlas_context direction:"in"` те же `guards:[…]` есть на каждой строке.

## Типичные промахи агента (бенчмарки Slotor)

| Промах | Почему | Как не повторить |
|---|---|---|
| Пропущен caller в `api/` при том же `create*` | Смотрели только первый путь / `partner` | После `called_by` читать **все** app-area |
| Пропущен `isLockTakeBonuses` и соседние гарды | Граф дал вызов, тело экшена не читали | `atlas_read` эталонный `action*` целиком |
| «Без указаний» = 0 Atlas | Bash доступен сразу, MCP — после ToolSearch | Скилл §0: сначала ToolSearch Atlas; хук `examples/hooks/php-search-hint.sh` + MCP `instructions` (см. `examples/README.md`) |
| Ложный unused | Пустой `called_by` без grep | `atlas_grep` / `withGrep` + views |
| Спека «битая» из‑за номера строки | Дрейф +74/+68 | Сверяй символ, не старый `file:line` |
| Шум AR magic в context | `@property` дублируются | Смотри `calls`/`called_by`, не весь dump класса |
| `atlas_batch` error | Нет поля `arguments` | Каждый call: `{tool, arguments:{…}}` |
| Vendor mutex / pastuhov = 0 | declaration_paths / exclude | Read файла или `include_vendor` |

## Areas (Slotor)

Обычные префиксы: `partner`, `admin`, `api`, `grcallback`, `console`, `frontend`,
`common`. Фильтр `area:` сужает search/grep/callers — **не** ставь его, если баг
может быть в другом приложении того же монолита.
