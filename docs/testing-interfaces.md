# Проверка HTTP, MCP и веб-интерфейса

Рабочая ветка: `feature/atlas-interfaces`, отдельный Git worktree. Адаптеры следуют общему `design.md`: HTTP вызывает `AtlasService`, бизнес-правила находятся в сервисах.

## TDD и выполненные проверки

1. До реализации написаны `tests/http.rs` и `web/model.test.mjs`.
2. Первый Rust запуск подтвердил отсутствие модулей. После подключения согласованных доменных типов и пустого Router выполнена поведенческая красная фаза: все четыре первоначальных HTTP-теста упали на 404 вместо ожидаемых результатов и кодов ошибок.
3. Модель интерфейса сначала содержала только незавершённые функции. Три из четырёх тестов упали с `not implemented`; проверка отказа некорректным датам уже проходила.
4. После реализации адаптеров четыре исходных HTTP-теста прошли. Добавлены два интеграционных сценария для полного цикла исправления/проверки и ошибок протокола; итог — шесть проходящих HTTP-тестов.
5. Четыре теста модели интерфейса прошли: UTC-период с включённой конечной календарной датой, ошибочные даты, пропуски дней на графике, различение одноимённых элементов и фильтрация.

Команды:

```sh
cargo test --offline --test http
node --test web/model.test.mjs
node --check web/app.js
node --check web/model.mjs
```

В исходной среде использованы отдельный `CARGO_TARGET_DIR` для этого worktree и общий подготовленный `CARGO_HOME`; это предотвращает смешивание артефактов одинакового crate из параллельных веток.

## Покрытое поведение

- MCP 2025-11-25: initialize, ping, tools/list, tools/call, notifications/initialized; ответы JSON-RPC и ошибки разделены с ошибками выполнения инструмента.
- Контекст содержит связанные сущности; только MCP-выдача контекста изменяет аналитику. Веб-чтение и обновление панели не учитываются.
- Токены агента и разработчика используются разными точками входа; агент не может принимать решения разработчика.
- Исправление агента видно в очереди; подтверждение повышает доверие до 3; последующее исправление агентом отклоняется.
- Origin, максимальный размер тела, версия протокола, аргументы и некорректный JSON проверяются на транспортной границе.
- Статическая страница, CSS и JS встроены в бинарник; внешние CDN и библиотеки не используются. Значения графа вставляются в DOM через textContent, а не как HTML.

## Границы проверки

Router-тесты выполняют настоящие HTTP-запросы in-process с memory repository. Проверки PostgreSQL и общесистемной индексации выполняются отдельно основной веткой.

Запуск отдельного HTTP preview на `127.0.0.1:18787` для браузерной проверки был предпринят, но sandbox запретил `TcpListener::bind` с `PermissionDenied / Operation not permitted`. Доступный Node-инструмент также завершил импорт Playwright ошибкой `does not provide an export named default`. Живой браузерный прогон и визуальная проверка скриншота не объявляются выполненными.

Данные аналитики обозначают успешно подготовленные к выдаче MCP-ответы, а не доказанное получение TCP-пакетов или использование данных в решении агента. Каждый запрос контекста получает самостоятельный ID события; внутри события сервис устраняет повторные ID элементов.

## Версия MCP

Совместимость закреплена на `2025-11-25`; современная ревизия `2026-07-28` не заявляется. JSON-ответы и отказ GET без SSE разрешены выбранной версией транспорта: [официальная спецификация](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports). Отличие от новой модели метаданных каждого запроса описано в [версировании MCP](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning).

## lean-mcp-responses

Change: `openspec/changes/lean-mcp-responses`. Поводом стала обратная связь по probe на `slotor777`. На голое имя поле находилось не полностью, поиск возвращал тела методов, а `atlas_context` был слишком большим.

1. Сначала написан `tests/lean_responses.rs` с 10 сценариями Service Layer. После добавления типов контракта и заглушек со старым поведением 7 тестов упали на assertions по ожидаемым причинам: нет фильтра kind и курсора, есть тяжёлая metadata, occurrence-вершины расходуют `max_nodes`, нет направления, диагностика отдаётся всегда. 3 теста-ограничителя регрессий прошли сразу.
2. В `tests/http.rs` добавлены 4 MCP/веб-сценария. 3 упали: в схеме нет новых аргументов, поиск без `next_cursor`, контекст отдаёт диагностику. Сценарий «веб отдаёт полные данные» прошёл сразу как ограничитель.
3. После реализации проходят `lean_responses` (10/10) и `http` (13/13).
4. `tests/workflow.rs` ожидал `returns` в MCP-контексте с аргументами по умолчанию. Это и есть BREAKING-изменение контракта, поэтому тест теперь явно передаёт `detail: "full"`.
5. Отклонение от TDD: SQL `NodeSearch` в `src/postgres.rs` написан вместе с memory-реализацией, до PostgreSQL-теста. Поэтому тест `postgres_search_filters_kinds_and_walks_keyset_cursor` дополнительно проверен на откат: без фильтров kind и курсора в SQL он падает (`[-0, -1]` вместо `[-0, -2]`), после восстановления SQL проходит.

Команды:

```sh
cargo test --locked                       # 117 passed, 5 ignored
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
node --test web/model.test.mjs web/indexing.test.mjs   # 11 passed
ATLAS_TEST_DATABASE_URL='postgresql://…/atlas_test' \
  cargo test --locked --test core_postgres -- --ignored --test-threads=1   # 4 passed (postgres:16-alpine)
openspec validate --all --strict --no-interactive
```

### Дополнение: бюджет occurrence-вершин

Замер на `slotor777` (331 867 вершин, 586 681 ребро; `processingGameTransaction`, depth=1, max_nodes=100) показал, что неограниченные occurrence-вершины раздувают ответ до 980 КБ (673 вершины). Тест `occurrence_sites_do_not_spend_the_node_budget` заменён двумя:

- `occurrence_sites_have_their_own_budget_and_callers_come_first` упал (23 occurrence-вершины при лимите 6), затем прошёл на отдельном пуле;
- `incoming_edges_are_admitted_before_outgoing_regardless_of_edge_ids` после введения пула упал (`missing call_site:K1:pay`), затем прошёл благодаря обходу входящих рёбер первыми.

Итог на `slotor777`:

| Запрос | Было (по отзыву) | Стало |
|---|---|---|
| context depth=0 | 85 КБ, из них 32 КБ диагностики | 0,7 КБ (`detail: full` + диагностика: 37,5 КБ) |
| context depth=1, both | 369 КБ, 2 из 8 вызывающих | 243 КБ, 8 из 8 |
| context depth=1, `direction: in` | — | 26 КБ, 8 из 8 |
| search `dep_tax_control`, `kinds: [write_site]` | 5 из 22 | 27 мест записи на одной странице |

## web-explore-and-reindex

Change: `openspec/changes/web-explore-and-reindex`.

1. `web/explore.test.mjs` (7 тестов) сначала запускался со stub-модулем, и все 7 упали на assertions. После реализации `explore.mjs` и `indexingView.reindex` проходят все 18 Node-тестов.
2. `tests/reindex.rs` (2 теста) и новые HTTP-сценарии (`/api/reindex`, `/api/search` с kinds/cursor, `/api/context` с параметрами) написаны до реализации. Red наблюдался только как ошибка компиляции из-за отсутствия API; поведенческий red для HTTP отдельно не зафиксирован. Маршруты `explore.mjs` и `icon.svg` добавлены в список публичных ресурсов после самих маршрутов.
3. Проверка в браузере (Chrome через Playwright) на `slotor777` с одноразовой PostgreSQL. Поиск `dep_tax_control` + «Записи» показывает 27 мест записи с выражением, методом-владельцем и условиями. Контекст `processingGameTransaction` сгруппирован: владелец, 8 вызывающих, вызовы, записи, чтения, возврат, ветки; длинные группы свёрнуты; переход к вызывающему и «Назад» работают. Кнопка поставила переиндексацию в очередь во время идущего прогона («Переиндексация в очереди»). Ошибок в консоли нет, `/favicon.ico` перенаправляется на `/assets/icon.svg`. Мобильная ширина 390 px — без горизонтальной прокрутки.

```sh
cargo test --locked                       # 123 passed, 5 ignored
node --test web/model.test.mjs web/indexing.test.mjs web/explore.test.mjs   # 18 passed
```

## agent-navigation

Change: `openspec/changes/agent-navigation`.

1. `tests/agent_navigation.rs` (5 сценариев MCP-08..11) и тест `Debouncer::pending` в `tests/project.rs` писались до реализации. После добавления типов контракта и заглушек упали все 5: поиск по символу совпадал как текст, `depth_limited` всегда был `false`, outline возвращал `NotFound`, `indexed_at` был пустым. Затем проходят 5 из 5 и 7 из 7.
2. `tests/http.rs`: новый сценарий MCP и обновлённый список инструментов упали (нет `atlas_outline`), затем прошли.
3. PostgreSQL (`postgres:16-alpine`): `postgres_symbol_search_and_index_metadata_round_trip` проходит. Всего 5 из 5. Red для SQL отдельно не наблюдался: тест написан одновременно с SQL.

Замер на `slotor777`:

| | Было | Стало |
|---|---|---|
| Структура `UserBonus` | context 44 КБ (по отзыву), depth=1 — 161 КБ | `atlas_outline` 5,4 КБ, 91 член |
| Места вызова `processingGameTransaction` | 11 (3 из них — `Logger::write*` со словом в тексте) | `mode: symbol` — 8, как у Serena |
| `truncated` при depth=0 | `true` | `false`, `depth_limited: true` |
| Свежесть | нет | `index.indexed_at`, `state`, `changes_pending` |

```sh
cargo test --locked    # 156 passed, 6 ignored
ATLAS_TEST_DATABASE_URL=… cargo test --locked --test core_postgres -- --ignored --test-threads=1   # 5 passed
```

## reset-atlas

Change: `openspec/changes/reset-atlas`. Решение владельца: полный сброс атласа и сразу полная переиндексация.

1. Тесты написаны до реализации: сервис (`reset_removes_graph_corrections_analytics_and_index_time`), HTTP (`reviewer_reset_requires_confirmation_and_queues_a_reindex`), PostgreSQL (`postgres_reset_clears_every_atlas_table`). Со заглушками без эффекта сервисный тест упал на проверках, HTTP — на 404. После реализации всё прошло; PostgreSQL 6 из 6.
2. Браузер (Chrome через Playwright) на `slotor777`: кнопка «Удалить граф», диалог подтверждения, затем сразу 0 вершин и пустой поиск. Переиндексация запустилась сама и восстановила 348 662 вершины, поиск снова находит результаты. Ошибок в консоли нет.

```sh
cargo test --locked    # 168 passed, 7 ignored
```
