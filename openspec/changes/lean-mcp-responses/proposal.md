# Proposal

## Why

Сравнение с Serena и grep на `slotor777` после `finer-graph-nodes` показало, что `atlas_search` полезен: он находит места вызова и записи вместе с условиями пути. Но пользоваться им мешают границы ответа:

- на голое имя `dep_tax_control` нашлось 5 записей из 22: лимит 100 заполнили места чтения, а фильтра по kind и пагинации нет;
- если поиск захватывает method или variable, в ответ попадают `returns` со snippet, `imports`, `assignments` и `usages`; при limit 40 ответ занял 230 КБ;
- `atlas_context` не помещается в контекст агента. При depth=0 ответ 85 КБ, из них 32 КБ диагностики; при depth=1 — 369 КБ, из них 131 КБ диагностики. Места чтения заняли 53 из 100 `max_nodes`, и в ответ попали только 2 из 8 вызывающих.

## What Changes

- `atlas_search`: фильтр `kinds`, курсорная пагинация (`cursor` → `next_cursor`) вместо единственной страницы до 100 результатов; ответ отдаёт компактную проекцию вершин без тяжёлых полей metadata.
- `atlas_context`: параметры `direction` (`both`/`in`/`out`), `detail` (`summary`/`full`, по умолчанию `summary`) и `include_diagnostics` (по умолчанию `false`).
- `atlas_context`: occurrence- и branch-вершины не расходуют `max_nodes`; их ограничивает бюджет рёбер.
- **BREAKING** для MCP-клиентов: по умолчанию в `atlas_context` нет диагностики и тяжёлых полей metadata. Прежний ответ можно получить через `detail: "full"` и `include_diagnostics: true`.
- Веб-API `/api/search` и `/api/context/{id}` сохраняют полные данные для проверяющего. Позже `web-explore-and-reindex` перевёл `/api/search` на компактные страницы и добавил параметры `/api/context`.

## Capabilities

### New Capabilities

Нет.

### Modified Capabilities

- `mcp-context`: проекции, фильтры, пагинация и бюджет узлов в ответах MCP.

## Impact

- `src/service.rs`, `src/domain.rs`, `src/repository.rs`, `src/postgres.rs`, `src/http.rs`.
- Контракт MCP `tools/list` и `tools/call` для `atlas_search` и `atlas_context`.
- Тесты `tests/core_behavior.rs`, `tests/http.rs`, `tests/core_postgres.rs`.
- Анализатор не меняется. Распознавание типов по `@var`, `@property` и связям ActiveRecord, а также связь `push(new Job)` с `Job::execute` вынесены в отдельный change.
