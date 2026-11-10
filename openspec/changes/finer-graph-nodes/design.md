# Design: finer-graph-nodes

## Context

Сейчас PHP-плагин кладёт в ноду метода полный `source`/`body`, а call/read/write оформляет рёбрами `from=method`. При `atlas_context(depth=0)` ответ уже десятки КБ; при depth≥1 — сотни КБ–МБ. Переменные конкурируют с callers за `max_nodes`. Grep видит закомментированные вызовы; atlas их отбрасывает на этапе лексера.

Владелец выбрал гранулярность C: occurrence + branch, плюс флаг `commented` на нодах.

## Goals / Non-Goals

**Goals**

- Method/function без полного текста тела.
- Вершины `branch` и occurrence (`call_site`/`write_site`/`read_site`/`return_site`) с коротким `expression` и `conditions`.
- Рёбра эффекта (`calls`/`reads`/`writes`) от occurrence к цели; method связан через `contains`.
- У всех вершин `metadata.commented: bool`.
- Индексация PHP-подобного кода внутри `//` / `#` / `/* */` как нод с `commented: true`.

**Non-Goals**

- Полный CFG / SSA.
- Отдельные ноды на каждое присваивание (assignments остаются в metadata variable, пока не потребуется иначе).
- Проекции MCP (`include_body`, фильтр kinds) — отдельный change; эта работа уменьшает сами ноды.
- Queue `push → execute` — отдельная задача Yii2.

## Architecture

```
Lexer ──► Unit(+comments) ──► Syntax(declarations)
                           ──► Flow(active body → branch/occurrence)
                           ──► CommentIndex(comment spans → commented nodes)
Project.nodes/edges ──► GraphSnapshot ──► AtlasService.context (без изменений контракта depth/max_nodes)
```

Зависимости: Flow и CommentIndex пишут только через `Project` (как сейчас). Service Layer не меняет семантику use-case, только получает более мелкий граф.

## Node / edge contract

| Kind | Содержимое metadata (минимум) | `commented` |
|------|-------------------------------|-------------|
| `method`/`function` | fingerprint, params, returns summary refs, declared_return_type, closure, namespace/imports — **без** `source`/`body` | false (active) или true (целиком в комментарии) |
| `branch` | `keyword`, `condition` (текст), `conditions` (накопленный путь), fingerprint | bool |
| `call_site` / `write_site` / `read_site` / `return_site` | `expression` (короткий фрагмент span), `conditions`, resolution/candidates где есть, fingerprint | bool |
| `variable` / `class` / … | как сейчас + `commented` | bool |

Рёбра:

- `contains`: method → branch, method → occurrence, branch → вложенный branch/occurrence (если occurrence создан внутри ветки — `contains` от ближайшего branch **или** от method; решение реализации: **от method + optional `parent_branch` в metadata occurrence**, чтобы не усложнять BFS; branch тоже `contains` от method).
- `calls`/`reads`/`writes`: occurrence → target (как сейчас на edges).
- Условия пути дублируются на occurrence (и на branch.condition); edge сохраняет `conditions` для совместимости.

ID occurrence: `call_site:{function_id}:{fingerprint}:{n}` (и аналоги), стабильно относительно текста выражения.

## Comment indexing

1. Лексер сохраняет spans комментариев (`Comment { start_byte, end_byte, line, text_without_markers }`), не эмитя их как обычные токены.
2. `CommentIndex` для каждого комментария:
   - если текст похож на объявление `function`/`class`/`public function` — попытка разобрать как declaration unit с `commented: true`;
   - иначе поиск вызовов/свойств простым повторным tokenize+expression parse фрагмента (best-effort);
   - при ошибке разбора — diagnostic `commented_parse_skipped`, без падения индекса файла.
3. Привязка scope: ближайшая объемлющая функция/класс по строкам файла; если нет — file-level orphan с `scope` в metadata.

Не пытаемся восстанавливать полное семантическое окружение внутри комментария (типы/`@var`) в первой итерации — достаточно occurrence + expression + commented.

## Patterns

- **Builder/emitter через Project** — уже есть; расширяем методами `occurrence_node`, `branch_node`.
- **Best-effort secondary pass** для комментариев (KISS): отдельный модуль `comments.rs`, не смешивать с основным CFG.
- YAGNI: не вводим CFG-edge `next`/`then`/`else` между branch, пока нет запроса.

## ENV

Новых переменных нет. Поведение включается анализом PHP по умолчанию. Некорректный ENV не затрагивается.

## Risks / Trade-offs

- Больше нод → `max_nodes` всё ещё может резать callers, если BFS без приоритета; mitigation в этом change: method без body уже сильно дешевле; приоритет обхода — follow-up.
- Comment parse даст ложные срабатывания на прозе в комментариях — ограничиваем эмит, если есть явный PHP-сигнал (`$`, `->`, `::`, `function`, `class`, `return`).
- **BREAKING** для потребителей `metadata.body`.

## Migration

Полная переиндексация проекта. Старые ноды с body заменяются snapshot publishing как обычно.
