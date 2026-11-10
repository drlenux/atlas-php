# Design: agent-navigation

## Architecture

```
MCP/web ─► AtlasService::{search_page, context_view, outline, freshness}
                 ├─ Repository: search_nodes(NodeSearch{mode}), adjacent_edges, node, meta/set_meta
                 └─ IndexingProgress (serve: state, changes_pending)
apply_snapshot ─► set_meta("indexed_at", now)        watcher ─► progress.set_changes_pending(debounce.pending())
```

Все правила — в Service Layer. HTTP добавляет к результату инструмента блок `index` из `AtlasService::freshness()`.

## Поиск по символу

`NodeSearch.mode: SearchMode::{Text, Symbol}`. `Symbol` совпадает без учёта регистра, если выполнено одно из условий:

- `name` объявления (без `$`) равен запросу (без `$`);
- `qualified_name` объявления равен запросу (`A\B::run`);
- `metadata.member_name` вершины места равен запросу (без `$`).

Текст выражения и строковые литералы не участвуют, поэтому `Logger::writeInfo('processingGameTransaction…')` не совпадает. Фильтр `kinds` и курсор работают так же. Для memory и PostgreSQL используются одинаковые условия.

## Усечение

`truncated` — бюджет вершин или рёбер не вместил найденное. `depth_limited` — у вершины на границе глубины есть рёбра в выбранном направлении к вершинам вне ответа. Оба поля — часть `GraphContext`.

## Outline

`AtlasService::outline(id) -> Outline`. Без вывода типов заново: всё берётся из графа.

- Класс, интерфейс, трейт, enum: `contains` → методы и свойства; `inherits` исходящие по цепочке — `parents`; `implements` и `uses_trait` исходящие — `interfaces`/`traits`; `inherits` и `implements` входящие — `children`/`implementations`.
- Метод: владелец; `overrides` исходящее — `overrides`, входящие — `overridden_by`.
- Член — одна строка: `L<line> <kind> <name>(<type $param>, …): <type> [magic] [annotation|framework]`. Для свойства — `: <type>`. Идентификаторы членов детерминированы (`method:<Class>::<name>`, `property:<Class>::$<name>`) и описаны в инструменте, поэтому их не нужно повторять в каждой строке.
- Ответ не учитывается в аналитике выдачи контекста: аналитика относится к `atlas_context`, а outline — справка о структуре.

## Свежесть

- Миграция: `atlas_meta(key text primary key, value text not null)`. `Repository::{meta, set_meta}`.
- `apply_snapshot` записывает `indexed_at` (UTC, секунды) в той же транзакции, что и публикация.
- `IndexingProgress` получает `changes_pending`: watcher выставляет его по `Debouncer::pending()` (изменение замечено, но debounce ещё не истёк).
- `Freshness { indexed_at: Option<i64>, state, changes_pending }`. В режиме `stdio` нет watcher: `state` — `waiting`, `changes_pending` — `false`, а `indexed_at` читается из БД.

`implementations` и `children` содержат только прямых наследников и реализации. Для `yii\db\ActiveRecordInterface` это `BaseActiveRecord`, а модели видны у него в `children`. Транзитивный список для базовых интерфейсов фреймворка занимал бы сотни строк, а структура задумана компактной.

## Patterns

- Read Model (`Outline`) — отдельная проекция графа для одного вопроса агента, без изменения хранения.

## ENV

Новых переменных нет. Задержку после сохранения задаёт `ATLAS_DEBOUNCE_MS`, по умолчанию 5000.

## Risks / Trade-offs

- `symbol` пропускает текстовые упоминания (ключи в строках, комментарии); для них остаётся `text`.
- `indexed_at` — время публикации, а не время чтения файлов. При `changes_pending: true` данные по изменённым файлам могут быть устаревшими.
