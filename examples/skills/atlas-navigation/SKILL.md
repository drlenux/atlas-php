---
name: atlas-navigation
description: PHP/Yii2 code graph via the Code Atlas MCP server (atlas_search, atlas_outline, atlas_context). Use to find who calls a method — including queue dispatch `push(new Job)` → `Job::execute` — what a method calls, where a field or ActiveRecord attribute is written and under which branch conditions, raw SQL tables touched by createCommand/findBySql, and a class's structure and hierarchy. Also lists what atlas cannot see, where Grep is still required (render()/view names, JS, translations, comments).
---

# Code graph navigation (Code Atlas)

Code Atlas indexes the PHP code (Yii2 plugin) into a graph: declarations
(classes, methods, properties) plus every **occurrence** — `call_site`,
`read_site`, `write_site`, `return_site`, `branch`, `sql_site` — each carrying
the chain of branch `conditions` under which it executes. It runs as the `atlas`
service in the consumer project's Compose stack (MCP on
`http://localhost:${ATLAS_HOST_PORT:-8080}/mcp`).

Setup: set `ATLAS_POSTGRES_PASSWORD`, `ATLAS_AGENT_TOKEN` and a different
`ATLAS_REVIEW_TOKEN` in `.env`, start the atlas service (`docker compose up -d atlas`),
then add an MCP server entry (see `examples/mcp.json.example` in the Code Atlas
repo) to your client's `.mcp.json` — either export `ATLAS_AGENT_TOKEN` in your
shell (many clients expand `${…}` in `.mcp.json` and do not read `.env`) or put
the token value there directly (keep `.mcp.json` gitignored). Restart the agent
session after changing MCP config.

Copy this skill into the consumer project as
`.claude/skills/atlas-navigation/SKILL.md` (or the equivalent path for your
agent tooling).

## Strategy (fewest calls)

1. **Find entry points by text** — domain strings, config keys, `.sh`/cron/yml,
   templates: `atlas_grep` (every text file; `path_glob` accepts `["*.php","!*/tests/*"]`),
   or `atlas_search(…, withGrep:true)` when you also want graph nodes.
2. **Callers / impact** — `atlas_context(entity_id, direction:"in")`: a callers tree,
   one line per caller with all its lines merged, `depth` = call levels (default 3).
   For direct callers only: `atlas_outline(…, sections:["called_by"])`.
3. **Read narrowly** — `atlas_read(path, start_line, end_line)` around the lines you
   need; `entity_id` reads the whole method, which can be 150+ lines.
4. **Batch independent lookups** — `atlas_batch(calls:[{tool, arguments}, …])` runs up
   to 10 read-only calls in one request (several outlines, a grep and a read).

Test code (`tests/`, `*Test.php`, `*Cest.php`) is left out of search, grep, callers
trees and `called_by` by default and counted as `excluded.tests=N`; pass
`include_tests:true` when tests matter. Scopes (what atlas indexes, greps, reads) are
set in `.atlas/setup.yaml`; `.env` files are never read.

- Applications are areas (`.atlas/setup.yaml`): e.g. `frontend`, `backend`,
  `console`, shared `common` — adapt names to the project. Lines show `[frontend]`
  after the location (callers-tree lines start with it); `area:"backend"` (or a list)
  limits `atlas_search`, `atlas_grep`, `atlas_outline` callers and `atlas_context`
  trees. `Yii::$app->x` is typed from the component config of the code's own
  application. Namespace-less classes are named `<area>:<Name>`
  (`class:console:m241121_…`, `class:api:Provider`).
- `undeclared` on a call site (diagnostic `undeclared_member`): the receiver class is
  fully known and no class, ancestor, trait, descendant, `__call` or Yii behavior
  declares the method — the call fails at runtime (e.g. a method removed in a
  refactoring). Search them with `atlas_search(query:"<name>", kinds:["call_site"])`.
- `[unresolved: name only]` in `called_by` / callers tree: a call through an untyped
  variable with the same method name — a possible caller to verify by reading.
  `possible.name_only=N` means the name is common and only the count is given.

## Which tool for which question

| Question | Call |
|---|---|
| Who calls method X (callers only, lean) | `atlas_outline(entity_id, sections:["called_by"])` |
| What method X calls (callees only) | `atlas_outline(entity_id, sections:["calls"])` |
| Method callers + callees (default outline) | `atlas_outline(method:…)` — omit `sections` |
| …up to the entry points (controller actions, jobs) | `atlas_context(entity_id, direction:"in", depth:3)` — callers tree |
| Who uses class C (callers of its members from outside) | `atlas_context(class:…)` — usage tree grouped by member |
| Does anything call interface method I::m? | `atlas_outline(method:I::m, sections:["called_by","overridden_by"])` — `called_by: none` is a definite answer; check implementations listed under `overridden_by` |
| Who dispatches job J to a queue | `atlas_outline(method:…::execute, sections:["called_by"])` — lines tagged `queue <name>` |
| Where field/AR attribute F is written, and in which branch | `atlas_search(query:"Owner::$F" or FQCN `app\\models\\Owner::$F`, kinds:["write_site"], mode:"symbol")`; page via `cursor` — bare `F` mixes every class |
| Where F is read | same with `kinds:["read_site"]`; `atlas_context(property:…, direction:"in")` mixes reads+writes and truncates at 100 |
| Who runs raw SQL against table T | `atlas_search(query:"table:public.T" or T, kinds:["table","sql_site"], mode:"symbol")`; follow `sql_reads` / `sql_writes` from `sql_site` (from `createCommand` / `findBySql` / `prepare` string args) |
| All call sites of a member, no false hits | `atlas_search(query:"name", kinds:["call_site"], mode:"symbol")` |
| Find a class by short name | `atlas_search(query:"PaymentGateway", kinds:["class"], mode:"symbol")` — never bare text without `kinds` |
| Class members, relations, parents, children | `atlas_outline(class:…)` |
| Overrides / overridden_by of a method | `atlas_outline(method:…, sections:["overrides","overridden_by"])` |
| Config param key / any text, comments, templates, `.sh`, cron, yml | `atlas_grep(query:"someConfigKey")` — every text file; then `atlas_read` for surrounding lines |
| Config key as graph node (if indexed) | `atlas_search(query:"someConfigKey", kinds:["config_key"], mode:"symbol")` |
| Read method body / file slice | `atlas_read(entity_id:"method:…")` or `atlas_read(path, start_line, end_line)` |
| Yii route / who links to it | `atlas_search(query:"site/index", kinds:["route","url_site"])`; `Url::to` and js/html path literals become `url_site` → `references` |
| Factory `new $class` from `const MAP = [… => Foo::class]` | typed via `class_string_values`; MAP keys are `read_site` |
| `Yii::createObject(X::class)` then `$x->method()` | result typed as `X`; outline `called_by` on `X::method` |
| `foreach ($q->batch() as $rows)` nested foreach | batch → `T[][]`, element typed as model |
| Calls inside `$fn = function () { … }` | method outline lists nested-closure call sites too |
| Yii `Event::on` / `$obj->on` / `trigger` | `kinds:["event"]`; literals ↔ `EVENT_*`; `off` → `unsubscribes` (handler for trigger remains) |

IDs are readable and can be built directly: `class:app\models\Order`,
`method:app\models\Order::pay`,
`property:app\models\Order::$amount`,
`table:public.wallet`,
`sql_site:method:…:<fingerprint>`.

- Prefer **narrow** graph arguments: `kinds`, `mode:"symbol"`, outline `sections`.
  For raw text use `atlas_grep` (not shell `rg`).
- `atlas_grep` header says `complete` or `next_cursor=…`, and `incomplete` when
  size/read problems skipped files (`# skipped …` lines); binaries are counted
  as `excluded.binary`. No match lines without
  `incomplete` means no match; do not treat missing graph edges as proof a symbol
  is unused (variable functions / magic methods may hide calls).
- `mode:"text"` (default) matches whole identifiers like `grep -w`
  (`getOrder` does not return `getOrderItem`) and ignores string literals
  (log messages mentioning a name); `mode:"substring"` for partial names;
  `mode:"symbol"` for exact declarations/members.
- `withGrep:true` appends `# grep supplement`: source lines matching the same
  query that atlas did **not** return (comments, strings, configs, unindexed
  code). It replaces a follow-up `atlas_grep`/`rg` for the same name.
- Results are ranked (exact name / declarations before occurrence sites) and paged:
  pass the header's `next_cursor=` back as `cursor` until it says `complete`.
- For callees prefer the method outline (`sections:["calls"]`, one line each)
  over `atlas_context(direction:"out")`.
- Do not call `atlas_correct` unless the user asks: it edits the index shared
  with everyone.

## Reading the results

All results are grep-style text, not JSON:

```
# index state=completed indexed_at=1794293641
# search nodes=2 complete
app/models/Order.php:120: call_site $this->charge() [resolved] if(!($this->status != $next)) member_name=charge receiver_class=app\models\Order | id=call_site:… rev=2
```

- `path:line[-end]: kind name`, then `[resolution]`, branch conditions as
  `if(...)` (outermost first), other fields as `key=value` (bare key = true),
  and after ` | ` the `id=` (for `atlas_context`/`atlas_outline`) and `rev=`
  (for `atlas_correct`).
- `atlas_context` numbers nodes `[1]` (root), `[2]`…; edges read
  `[5] -calls-> [1] path:line [resolution] …`. `omitted_members=N` in the
  header: the root's members are not listed, use `atlas_outline`.
- `atlas_read`: `# path:start-end`, then `line: text`.

- `resolution` on a call/edge: `resolved` — from code types; `annotated` — only
  via PHPDoc (`@var`/`@property`/library docs), **verify by reading the code
  before relying on it**; `candidate` — several possible targets;
  `unresolved`/`external` — target unknown or a PHP built-in.
- Outline member tags: `magic` (AR column or relation, not a real PHP property),
  `annotation` (from `@property`), `framework` (Yii relation derived from
  `getXxx(): ActiveQuery`).
- `conditions` are raw PHP expressions from the outermost branch inward; `!(…)`
  means the else-branch. Read them to answer "when does this write happen".

## Freshness

The first line of every result is `# index state=… indexed_at=…`, with
`changes_pending pending=a.php,b.php` while edits are not yet indexed.
An edit reaches the index in ~10 s (5 s debounce + incremental rebuild), and the
dependent edges in other files are updated too. If a file you just edited is in
`pending=`, or `state` is not `completed`, answer questions about that file
with Read/Grep instead of atlas.

## Use Grep instead (atlas is blind here)

- **View names / `render()` / `renderPartial()` / `renderAjax()`:** PHP inside
  `**/views/**` *is* indexed (reads/writes/calls in the template body), but
  atlas does not resolve which controller/action renders a given view or
  partial — find those call sites with Grep.
- **Translations** (`**/messages/**`), **comments and docblocks**. Path literals
  in `.js`/`.html`/`.vue` become `url_site` when they match a known route; other
  JS logic is not typed.
- **Configs:** keys in `**/config/params.php` are indexed as `config_key`; any
  other text is available via `atlas_grep` (respects project excludes, so
  `*-local.php` is typically not scanned; `excluded.*=N` in the header counts
  files left out by scope).
- **Type gaps:** some factory/`catch` paths stay `unresolved`, so a method
  missing from `called_by` is not proof it is unused — confirm with `atlas_grep`
  before deleting or changing a signature. Variable functions and magic
  `__call`/`__get` are not fully resolved statically.
- Renaming a symbol across references: atlas is read-only; use your IDE/LSP
  rename or edit after `atlas_grep`.
