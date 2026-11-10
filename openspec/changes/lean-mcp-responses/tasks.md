# Tasks: lean-mcp-responses

## 1. Поиск

- [x] 1.1 Тест: `kinds` фильтрует выдачу; невалидный kind отклоняется
- [x] 1.2 Тест: курсорная пагинация обходит все совпадения без повторов, `next_cursor` равен null на последней странице
- [x] 1.3 Тест: поиск отдаёт summary-проекцию без `returns`/`imports`/`assignments`/`usages`/`fingerprint`
- [x] 1.4 `NodeSearch` в Repository (memory + PostgreSQL), `AtlasService::search_page`

## 2. Контекст

- [x] 2.1 Тест: `direction: in` отдаёт вызывающих через call_site без собственных эффектов
- [x] 2.2 Тест: `direction: out` отдаёт эффекты без вызывающих
- [x] 2.3 Тест: occurrence-вершины не расходуют `max_nodes`
- [x] 2.4 Тест: диагностика по умолчанию не отдаётся, `include_diagnostics` её включает
- [x] 2.5 Тест: `detail: summary` по умолчанию, `full` возвращает тяжёлые поля
- [x] 2.6 `ContextRequest`, `AtlasService::context_view`

## 3. MCP и веб

- [x] 3.1 Тест: схема `tools/list` содержит новые аргументы; неизвестные и невалидные отклоняются
- [x] 3.2 Тест: веб-API по-прежнему отдаёт полные данные и диагностику
- [x] 3.3 HTTP-адаптер, описание инструментов

## 4. Проверка

- [x] 4.1 `cargo test --locked`, `cargo fmt --check`, clippy `-D warnings`
- [x] 4.2 PostgreSQL-интеграция для `NodeSearch` (если доступна база)
- [x] 4.3 `openspec validate lean-mcp-responses --strict`
