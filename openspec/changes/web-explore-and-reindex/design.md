# Design: web-explore-and-reindex

## Context

Веб-интерфейс — поставляемые `web/index.html`, `app.js` и `model.mjs`. Логика представления живёт в `model.mjs` и тестируется Node test runner, а `app.js` только связывает DOM и API. Watcher (`main.rs::spawn_watcher`) раз в `ATLAS_POLL_MS` сканирует файлы и при изменении сигнатуры, после debounce, выполняет `prepare` и `publish`. `IndexingProgress` — общее состояние индексации: его читает `/api/indexing` без блокировки сервиса и обновляет индексатор.

## Goals / Non-Goals

**Goals**

- Проверяющий читает поиск, контекст и аналитику без технических id.
- Проверяющий может вручную запустить полную переиндексацию и видит, что запрос принят.

**Non-Goals**

- Отмена или прерывание идущей индексации.
- Переиндексация отдельных файлов.
- Граф-визуализация контекста (SVG/canvas): группировка списком решает задачу проще.

## Architecture

```
[Кнопка] ─POST /api/reindex─► IndexingProgress.request_reindex()  (без блокировки AtlasService)
watcher loop: changed = debounce.observe(..); if progress.take_reindex_request() || changed → prepare/publish
/api/indexing ─► snapshot { …, reindex_queued }
```

Запрос переиндексации хранится в `IndexingProgress`. Это единственное общее состояние, которое HTTP и watcher уже разделяют без блокировки сервиса: публикация держит блокировку сервиса долго, а кнопка должна отвечать сразу. Отдельный тип-триггер потребовал бы ещё одного канала через `router()` ради одного флага (KISS).

Семантика:

- `POST /api/reindex` → `202 {"reindex_queued": true}`. Запрос идемпотентен: повторное нажатие до начала прогона не ставит второй прогон.
- Если индексация уже идёт, флаг остаётся установленным, и следующий цикл watcher запустит ещё один полный прогон. Так учитываются изменения, сделанные во время текущего прогона.
- `take_reindex_request()` атомарно читает и сбрасывает флаг.
- В режимах `index`/`stdio` watcher нет; кнопка работает только с `serve`.

## Web API

- `GET /api/search?q=&kinds=a,b&cursor=&limit=` → `SearchPage` с `Detail::Summary`. Kinds проверяет Service Layer.
- `GET /api/context/{id}?depth=&max_nodes=&direction=&detail=&include_diagnostics=`. По умолчанию прежние значения: depth `min(2, max)`, `max_nodes` из конфигурации, `both`, `full`, диагностика включена. Границы те же, что у MCP.
- Интерфейс запрашивает контекст с depth=1, `detail=summary` и диагностикой. Кнопка «Полные данные (JSON)» загружает depth=0 с `detail=full` для выбранной вершины.

## Модель представления (`model.mjs`)

- `ownerOf(id)`: извлекает метод-владелец из id occurrence/branch вида `<kind>:<function_id>[:<kind>]:(php-struct-v1|comment):…`. Для прочих id возвращает `null`.
- `entityLabel(entity)`: для occurrence/branch заголовок — выражение (`name`/`label`), подзаголовок — владелец; для объявлений — `qualified_name`. Используется в поиске, контексте и аналитике.
- `contextView(context)`: группирует граф относительно корня — вызывающие (`calls` → корень, вместе с call_site и его владельцем), владелец (`contains` → корень от не-occurrence), эффекты (`contains` корень → site → `calls`/`writes`/`reads`, плюс `return_site`), ветки, прочие связи. Условия берутся из metadata site.
- `indexingView`: поле `reindex` (`{enabled, label}`): кнопка недоступна без связи и при уже поставленном запросе; во время прогона подпись «Переиндексировать после текущего прогона».

## Patterns

- Presentation Model — `model.mjs` уже так устроен. Новые функции чистые и тестируются без DOM.
- Без новых абстракций на Rust-стороне: флаг в существующем общем состоянии, параметры веб-API разбираются в существующий `ContextRequest`/`NodeSearch`.

## ENV

Новых переменных нет.

## Risks / Trade-offs

- `ownerOf` разбирает формат id PHP-плагина. Если формат изменится, функция вернёт `null`, и интерфейс покажет путь и строку без владельца. Тест закрепляет текущий формат.
- Кнопку может нажать любой, у кого есть токен проверяющего. Полный прогон на большом проекте занимает минуты (`slotor777` — около 2,5 мин). Защита от повторов — идемпотентный флаг.
