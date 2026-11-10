# Design: lean-mcp-responses

## Context

После `finer-graph-nodes` граф состоит из мелких вершин, но размер ответа MCP по-прежнему задают три вещи:

1. поиск отдаёт вершины целиком, вместе с `returns[].snippet`/`origins`, `imports`, `assignments` и `usages`;
2. контекст всегда добавляет до 1000 диагностик по всем затронутым файлам;
3. BFS контекста считает каждую occurrence-вершину в `max_nodes`, поэтому собственные `read_site` метода вытесняют вызывающих.

## Goals / Non-Goals

**Goals**

- Агент находит все места записи поля при запросе по голому имени: фильтр `kinds` и перебор страниц.
- Ответы поиска и контекста по умолчанию компактны; полные данные можно запросить явно.
- Вызывающих метода можно получить отдельно от его собственных эффектов (`direction: "in"`).

**Non-Goals**

- Изменения анализатора: `@var`/`@property`, связи ActiveRecord, `queue->push(new Job)` → `Job::execute`. Это отдельный change.
- Семантический поиск и ранжирование. Порядок выдачи остаётся по `id`, что позволяет использовать стабильный keyset-курсор.
- Удаление строковых литералов из сопоставления. Строка вида `updateAll(['dep_tax_control' => 0])` — реальная запись колонки. Если игнорировать литералы, такие записи пропадут без следа, а лишнее совпадение агент увидит сам. Против шума работают `kinds` и поля `member_name`/`resolution` в компактной проекции.

## Architecture

```
MCP atlas_search ─► SearchArgs ─► AtlasService::search_page(NodeSearch) ─► Repository::search_nodes(NodeSearch)
                                         └─ NodeProjection::Summary
MCP atlas_context ─► ContextArgs ─► AtlasService::context_view(ContextRequest) ─► Repository::node / adjacent_edges
                                         ├─ Direction filter, NodeBudget (occurrence free)
                                         └─ NodeProjection, diagnostics opt-in
Web /api/search, /api/context ─► те же use-case с Detail::Full и диагностикой (поведение проверяющего не меняется)
```

Правила проекции и бюджета находятся в Service Layer и общие для всех входов. HTTP разбирает аргументы и проверяет границы. Repository отвечает только за фильтрацию по тексту, kind и курсору.

## Contracts

### atlas_search

| Аргумент | Тип | По умолчанию | Проверка |
|---|---|---|---|
| `query` | string | — | обязателен |
| `kinds` | string[] | все | 1..20 элементов, каждый `[a-z0-9_]+` |
| `limit` | integer | 20 | 1..100 |
| `cursor` | string | начало | `next_cursor` предыдущей страницы |

Ответ: `{"nodes": [...summary], "next_cursor": string|null}`. Выдача упорядочена по `id`. Страница возвращает вершины с `id > cursor`. Чтобы определить `next_cursor`, repository читает `limit + 1` строк. Курсор — `id` последней вершины страницы. Он стабилен между переиндексациями: удалённые вершины пропускаются, новые с большим `id` попадают на следующие страницы.

### atlas_context

| Аргумент | Тип | По умолчанию |
|---|---|---|
| `direction` | `both` \| `in` \| `out` | `both` |
| `detail` | `summary` \| `full` | `summary` |
| `include_diagnostics` | boolean | `false` |

- `in` проходит только по рёбрам, где текущая вершина — `to`, а `out` — только по рёбрам, где она `from`. Правило «occurrence раскрывает один hop» сохраняется и действует в выбранном направлении. При `in` от метода цепочка `M ←calls─ call_site ←contains─ caller` даёт вызывающих, а при `out` цепочка `M ─contains→ site ─calls/reads/writes→ target` даёт эффекты.
- **NodeBudget**: у вершин kind `branch`, `call_site`, `read_site`, `write_site` и `return_site` свой пул размером `max_nodes`, отдельный от пула объявлений. Поэтому места чтения не вытесняют методы-вызывающих, а ответ остаётся ограниченным: не больше `2 × max_nodes` вершин. Входящие рёбра вершины обходятся раньше исходящих, и места вызова у вызывающих попадают в пул раньше собственных occurrence метода. Если пул исчерпан, `truncated = true`.
- Первая версия вообще не ограничивала occurrence-вершины. На `slotor777` (`processingGameTransaction`, depth=1) ответ вырос до 980 КБ и 630 occurrence-вершин, поэтому пул ограничен.
- Если `include_diagnostics = false`, поле `diagnostics` — пустой массив. Форма ответа не меняется.

### Проекция `summary`

Из `metadata` вершин и рёбер удаляются ключи `fingerprint`, `returns`, `imports`, `assignments`, `usages`, `body` и `source`. Остальные поля сохраняются: `id`, `kind`, `name`, `qualified_name`, `span`, `trust`, `revision` и прочая metadata, включая `expression`, `conditions`, `resolution`, `member_name`, `type`, `parameters` и исправления агента. Используется denylist: небольшие поля плагинов и исправления уровня 2/3 остаются видны без перечисления. Тяжёлые поля перечислены явно, и их список проверяется тестом.

`revision` в проекции позволяет вызвать `atlas_correct` без запроса `full`.

## Patterns

- **Parameter Object** (`NodeSearch`, `ContextRequest`) — число опций поиска и контекста выросло. Объект параметров заменяет длинные позиционные сигнатуры и даёт единые значения по умолчанию для MCP и веба.
- **Projection** — функция `project_node`/`project_edge` в Service Layer. Отдельный DTO-тип не нужен (YAGNI): форма `Node` сохраняется, меньше становится только `metadata`.
- Существующие `search(query, limit)` и `context(id, depth, limit)` остаются короткими обёртками с полными данными для веба и тестов. Логика при этом не дублируется: они вызывают те же `search_page`/`context_view`.

## ENV

Новых переменных нет. `ATLAS_MAX_CONTEXT_NODES` и `ATLAS_MAX_CONTEXT_DEPTH` сохраняют прежний смысл. Occurrence-вершины не считаются в `max_nodes`.

## Risks / Trade-offs

- **BREAKING** по умолчанию для MCP-клиентов, которые ожидали `diagnostics` или `returns` в контексте. Смягчение: явные `detail: "full"` и `include_diagnostics: true`; описание инструмента это указывает.
- Порядок по `id` группирует выдачу по префиксу kind. Ранжирования нет, но с `kinds` и курсором выдачу можно перебрать полностью.
- Отдельный пул удваивает верхнюю границу числа вершин. Агенту, которому нужны все эффекты метода, нужен `direction: out` или поиск `kinds` + `cursor`; контекст остаётся обзором.

## Migration

Переиндексация не нужна: меняется только чтение.
