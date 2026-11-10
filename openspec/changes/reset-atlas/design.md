# Design: reset-atlas

## Architecture

```
[Удалить граф] ─confirm()─► POST /api/atlas/reset {"confirm":"reset"}
  HTTP: проверка тела ─► service_call(AtlasService::reset) ─► transactional(Repository::reset)
        ─► IndexingProgress::request_reindex() ─► 200 {"reset":true,"reindex_queued":true}
watcher: take_reindex_request() ─► полный prepare/publish в пустой атлас
```

- `Repository::reset()` очищает всё хранимое состояние атласа: memory — `MemoryState::default()` в транзакции; PostgreSQL — `TRUNCATE atlas_edges, atlas_nodes, atlas_corrections, atlas_diagnostics, atlas_deliveries, atlas_meta` в транзакции.
- Сброс ждёт блокировку сервиса. Если идёт публикация, сброс выполнится после неё, а затем поставленная в очередь переиндексация заново построит граф. Если подготовка снимка уже идёт вне блокировки, её публикация после сброса просто заполнит пустой атлас; поставленная переиндексация всё равно выполнится.
- Подтверждение в теле запроса защищает от случайного вызова (повтор ссылки, ошибка клиента). Права даёт токен проверяющего, разрешённые источники — Origin, как у остальных `/api/*`.

## Patterns

Новых паттернов нет: use case в Service Layer, адаптер HTTP, та же очередь переиндексации.

## ENV

Новых переменных нет.
