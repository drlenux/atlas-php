# Code Atlas

Code Atlas строит карту PHP/Yii2-проекта: какие классы и методы существуют, кто кого вызывает, откуда читаются данные, куда они записываются и при каких условиях выполняются операции. Агент получает эту информацию через MCP, разработчик просматривает граф, проверяет исправления и смотрит аналитику в браузере.

Анализаторы написаны на Rust и встроены в приложение. Исходники исследуемого проекта читаются как текст: PHP CLI, Composer, автозагрузчик и запуск приложения для анализа не нужны. В поставляемом контейнере работают Atlas и PostgreSQL с расширением pgvector. Векторный поиск в текущем коде не используется.

Подробное объяснение всего пути — от чтения файлов до ответа агенту — находится в [How works.md](How%20works.md).

## Что умеет текущая версия

- Строит граф объявлений, наследования, вызовов, чтений, записей, переменных, параметров и ветвлений; сохраняет позиции и условия выполнения.
- Выводит типы из PHP-кода, PHPDoc, возвратов функций и статически понятных механизмов Yii2. Различает установленную цель, аннотацию, кандидатов и неизвестную цель.
- Извлекает raw SQL из `createCommand` / `findBySql` / `prepare` в PHP (`sql_site`, таблицы, `sql_reads`/`sql_writes`); при `languages: […, pgsql]` индексирует `.sql`/`.pgsql` (DDL schema/table/column/view/function/index).
- Дополняет PHP-граф сведениями о компонентах Yii2, behaviors, magic properties, ActiveRecord/ActiveQuery, конфигурационных ключах, представлениях, URL, очередях и событиях — в пределах поддержанного статического разбора.
- Предоставляет семь MCP-инструментов: поиск сущностей, контекст, структуру класса/метода, текстовый поиск, чтение строк, исправление знаний и пакет запросов.
- Следит за изменениями файлов, выполняет инкрементальные обновления и показывает состояние индексации. Полный проход доступен вручную.
- Сохраняет исправления агента отдельно от результатов анализа, поддерживает проверку разработчиком и три уровня доверия.
- Показывает в веб-интерфейсе поиск и связи кода, очередь проверки и статистику выдачи графа через MCP.

Полнота графа зависит от доступных исходников и возможностей парсера. Пустой список вызывающих сам по себе не доказывает, что код не используется.

## Быстрый запуск в проекте

Нужны Docker с Compose, доступный образ Atlas и каталог исследуемого проекта. Сборка образа описана ниже; наличие примера тега в репозитории не подтверждает его публикацию в реестре.

1. Добавьте сервис и volume из [examples/compose.atlas.yml](examples/compose.atlas.yml) в Compose исследуемого проекта. Для отдельного запуска можно использовать [docker-compose.yml](docker-compose.yml) этого репозитория, указав `ATLAS_HOST_PROJECT_ROOT`.
2. Возьмите [.env.example](.env.example) за основу. Задайте `ATLAS_IMAGE` доступным тегом или digest, заполните `ATLAS_POSTGRES_PASSWORD` и два **разных** токена: `ATLAS_AGENT_TOKEN` и `ATLAS_REVIEW_TOKEN`. Для каждого секрета можно отдельно выполнить `openssl rand -hex 32`.
3. Создайте в исследуемом проекте `.atlas/setup.yaml` по [examples/setup.yaml](examples/setup.yaml). Проверьте каталоги, исключения, версию PHP и список приложений `areas` под свой проект. В шаблоне есть примерные исключения и приложения; их следует адаптировать.
4. Запустите сервис из каталога его Compose-файла:

```sh
docker compose config --quiet
docker compose pull atlas
docker compose up -d --no-build atlas
docker compose logs -f atlas
```

По умолчанию веб доступен на `http://localhost:8080`, MCP — на `http://localhost:8080/mcp`. При другом `ATLAS_HOST_PORT` используйте соответствующий порт. На веб-странице введите reviewer-токен; он хранится в памяти открытой страницы. HTTP MCP принимает `Authorization: Bearer <ATLAS_AGENT_TOKEN>`.

Для агента: шаблон MCP-сервера — [examples/mcp.json.example](examples/mcp.json.example); skill навигации — [examples/skills/atlas-navigation/](examples/skills/atlas-navigation/) (`SKILL.md` + `reference.md`; скопируйте каталог в `.claude/skills/atlas-navigation/` исследуемого проекта).

Первичная индексация запускается автоматически. Доступность HTTP и успешный healthcheck ещё не означают готовность графа: дождитесь завершения публикации в панели или сообщения `100% committed` в логах.

Проект монтируется в `/project` только для чтения. Данные PostgreSQL сохраняются в named volume `atlas-postgres`. Compose публикует порт Atlas на `127.0.0.1`, PostgreSQL слушает внутри контейнера. Пользователь процесса Atlas должен иметь права читать примонтированные файлы.

## Настройки проекта

`.atlas/setup.yaml` читается при старте. Все ключи необязательны; отсутствующие получают значения из кода. Неизвестные ключи и недопустимые значения останавливают запуск. Полный шаблон с комментариями — [examples/setup.yaml](examples/setup.yaml), проверка значений — [src/config.rs](src/config.rs) и [src/setup.rs](src/setup.rs).

Минимальный пример:

```yaml
version: 1
exclude: [runtime, "*/web/assets"]
index:
  watch: [common, frontend, backend]
  include_vendor: false
  declaration_paths: [vendor/yiisoft]
  languages: [php]
  frameworks: [yii2]
  php_version: "8.2"
```

| Раздел | Назначение и значения по умолчанию |
| --- | --- |
| `exclude` | Общие исключения: граф, текстовый поиск и чтение файлов. По умолчанию пусто. |
| `index.watch`, `index.exclude` | Каталоги анализа (`["."]`) и дополнительные исключения только для графа (пусто). |
| `index.include_vendor` | `false`; `true` включает полный анализ vendor. |
| `index.declaration_paths` | `[vendor/yiisoft]`: объявления библиотек доступны без анализа тел при выключенном полном vendor. |
| `index.languages`, `index.frameworks` | `[php]`, `[yii2]`; добавьте `pgsql` для `.sql`/`.pgsql`; `frameworks: []` отключает Yii2. SQL внутри PHP разбирается и при одном только `php`. |
| `index.php_version` | `"8.2"`; допустимы 5.6, 7.0–7.4, 8.0–8.5. Выбор версии не означает полного покрытия её синтаксиса. |
| `index.max_file_bytes` | `2097152`: до 2 MiB на файл анализа. |
| `index.max_analysis_bytes` | `67108864`: до 64 MiB суммарного текста за проход. |
| `index.max_graph_elements` | `2000000`: вершины, рёбра и диагностики. |
| `index.analysis_timeout_secs` | `120`: ограничение времени анализа, проверяемое внутри конвейера. |
| `index.debounce_ms`, `index.poll_ms` | `5000`, `1000`: ожидание после последней правки и период сканирования. |
| `grep.exclude`, `grep.max_file_bytes` | Исключения только текстового поиска; лимит файла `16777216` (16 MiB). |
| `tests.paths` | `["tests/*", "*/tests/*", "*Test.php", "*Cest.php"]`: тесты скрыты от агентского поиска и списка вызывающих без `include_tests: true`. |
| `mcp.max_context_nodes`, `mcp.max_context_depth` | `100`, `3`: верхние пределы запроса контекста. |
| `mcp.max_request_bytes` | `1048576`: до 1 MiB на запрос. |
| `areas` | По умолчанию пусто. Приложения по папкам, метки и фильтр `area`, контексты компонентов Yii2. |

Общие исключения действуют для всех способов доступа. `index.exclude` оставляет файл доступным текстовому поиску; `grep.exclude` не убирает его из графа. Встроенно исключены `.git`, `.idea`, `.vscode`, `.agents`, `.codex`, `.claude`, `.atlas`, `node_modules`, `target`, файлы `.env` и `.env.*`. Символические ссылки не обходятся.

В `areas` задаются имена приложений и их пути:

```yaml
areas:
  frontend: { paths: [frontend] }
  backend: { paths: [backend] }
  common: { paths: [common], shared: true }
```

Выдача получает метки `[frontend]`, `[backend]`, `[common]`. Классы и функции без namespace внутри области получают префикс, например `frontend:Controller`, чтобы разные приложения не смешивались. Для вложенных путей побеждает самый длинный подходящий путь. `shared: true` обозначает общую область; Yii2 учитывает её при выборе конфигурации приложения.

При наличии setup-файла старые переменные анализа (`ATLAS_WATCH_DIRS`, `ATLAS_EXCLUDE` и другие перенесённые ключи) запрещены: ошибка перечислит конфликтующие имена. Без setup-файла сохранён прежний путь через ENV. Старые настройки PHP-процессов (`ATLAS_PHP_BINARY`, `ATLAS_PHP_PLUGIN`, `ATLAS_PHP_LINT` и связанные ключи) удалены и вызывают ошибку запуска.

## Окружение и ресурсы

| Переменная | Назначение |
| --- | --- |
| `ATLAS_IMAGE` | Образ для Compose. Сейчас Compose без ENV выбирает `drlenux/memory-tools:0.1.0`, а `.env.example` содержит `0.1.25`. Укажите нужный доступный тег или digest явно. |
| `ATLAS_POSTGRES_DB`, `ATLAS_POSTGRES_USER`, `ATLAS_POSTGRES_PASSWORD` | Настройки встроенной БД в Compose: `atlas`, `atlas`, обязательный пароль. Передаются также как `POSTGRES_*` и `PG*`. |
| `ATLAS_HOST_PROJECT_ROOT`, `ATLAS_HOST_PORT` | Настройки Compose: каталог хоста (`.`) и порт (`8080`). |
| `ATLAS_DATABASE_URL` | Обязательное подключение приложения к PostgreSQL: URI или libpq conninfo. В Compose по умолчанию `host=127.0.0.1 connect_timeout=5`, остальные параметры берутся из `PG*`. |
| `ATLAS_PROJECT_ROOT`, `ATLAS_BIND` | Корень внутри процесса и адрес HTTP. Compose задаёт `/project` и `0.0.0.0:8080`; локальный bind по умолчанию `127.0.0.1:8080`. |
| `ATLAS_AGENT_TOKEN`, `ATLAS_REVIEW_TOKEN` | Обязательные непустые разные токены MCP и reviewer API. |
| `ATLAS_CPU_LIMIT`, `ATLAS_MEMORY_LIMIT_MB` | `1` CPU и `1024` MiB по умолчанию. Compose ограничивает весь контейнер, включая PostgreSQL. Сами ENV вне Compose не создают системных ограничений. |
| `ATLAS_INDEX_WORKERS` | `0`: автоматически `ceil(ATLAS_CPU_LIMIT)` в пределах 1..32; 1..32 — явное число рабочих потоков. |
| `ATLAS_ALLOWED_ORIGINS` | JSON-массив разрешённых HTTP(S) origins. Compose подставляет localhost и 127.0.0.1 с внешним портом; для reverse proxy задайте реальные origins. |

Независимые задачи чтения, PHP-разбора, анализа функций и часть Yii2-обработки выполняются ограниченным числом потоков. Общие этапы и публикация остаются последовательными. Увеличение workers может повысить расход памяти; лимиты следует выбирать для Atlas и БД вместе.

После изменения setup-файла перезапустите сервис. После изменения ENV пересоздайте контейнер:

```sh
docker compose up -d --no-build --force-recreate atlas
```

## Индексация и обслуживание

Watcher опрашивает содержимое файлов и ждёт `debounce_ms` после последнего изменения. Во время работы процесса он может обновлять изменённые файлы и зависимые участки. Обнаружение файлов, чтение и сбор объявлений по-прежнему охватывают всю выбранную область; сокращаются анализ тел функций и публикация. После перезапуска базовое состояние инкрементального анализа строится заново полным проходом.

Панель показывает этап, текущие файлы, счётчики, занятые потоки, прошедшее время и очередь следующего запуска. Процент и ETA относятся к текущему этапу. При неизвестном объёме работ процент отсутствует. Успех фиксируется после транзакционной публикации. При ошибке предыдущий опубликованный граф сохраняется.

- «Переиндексировать» — `POST /api/reindex` с reviewer-токеном: ставит полный проход в очередь. Повторные запросы объединяются; запрос во время работы ставит следующий проход. Требуется работающий `serve`.
- Состояние — `GET /api/indexing` с reviewer-токеном, включая `reindex_queued`.
- «Удалить граф» — `POST /api/atlas/reset` с reviewer-токеном и `{"confirm":"reset"}`: удаляет граф, диагностики, исправления, подтверждения, аналитику и время индексации; ставит полный проход в очередь. Эти сведения восстановить переиндексацией нельзя, кроме автоматически вычисляемого графа.

Для обновления самого приложения нужен новый образ: HTML/CSS/JS встроены в бинарник. Перезапуск старого образа не обновляет код. Порядок сборки, установки, обновления и резервного копирования — [docs/distribution.md](docs/distribution.md). Named volume содержит также ручные знания и историю; удаление volume не является обычным шагом обновления.

## Подключение MCP

HTTP и stdio используют общий JSON-RPC адаптер с закреплённой версией `2025-11-25`. HTTP принимает один JSON-RPC объект на POST, возвращает JSON; уведомления получают HTTP 202. GET-поток SSE не реализован.

Для ручного HTTP-клиента нужны заголовки:

```text
Authorization: Bearer <ATLAS_AGENT_TOKEN>
Content-Type: application/json
Accept: application/json, text/event-stream
MCP-Protocol-Version: 2025-11-25
```

Первый запрос `initialize` может не содержать `MCP-Protocol-Version`; версия обязательна у следующих запросов. Поддерживаются `initialize`, `ping`, `tools/list`, `tools/call` и уведомления. Конкретные схемы аргументов сообщает `tools/list`.

| Инструмент | Когда использовать | Основные аргументы |
| --- | --- | --- |
| `atlas_search` | Найти сущности графа | `query`; `mode: text/substring/symbol`, `kinds`, `limit` 1..100, `cursor`, `withGrep`, `grepCursor`, `area`, `include_tests`. |
| `atlas_context` | Получить вызывающих или локальные связи | `entity_id`; `direction: in/out/both`, `depth`, `max_nodes`, `detail: summary/full`, `include_diagnostics`, `area`, `include_tests`. |
| `atlas_outline` | Посмотреть структуру и прямые вызовы | `entity_id`; `sections`, `area`, `include_tests`. Например `sections: ["called_by"]`. |
| `atlas_grep` | Искать текст в текущих исходниках | `query`; `regex`, `case_sensitive`, `word`, `path_prefix`, `path_glob`, `include_vendor`, `output`, `context`, `limit` 1..200, `cursor`, `area`, `include_tests`. |
| `atlas_read` | Прочитать текущие строки исходника | `entity_id` или `path` со `start_line/end_line` либо `around_line/context`; до 400 строк. |
| `atlas_correct` | Исправить существующее знание | `kind: node/edge`, `id`, `expected_revision`, `patch`, `reason`. |
| `atlas_batch` | Выполнить 1..10 запросов одним обращением | `calls: [{tool, arguments}]`; разрешены пять инструментов чтения, без correct и вложенного batch. |

Результат инструмента — компактный текст в MCP `content`, а не JSON-сериализация графа. В начале указывается состояние индекса (`# index`), далее строки с путями, номерами строк, ID и ревизиями. HTTP API веба возвращает JSON.

`text` ищет целые идентификаторы, `substring` — части имён, `symbol` — точное объявление или член. Веб-поиск по умолчанию использует `substring`, MCP — `text`. `withGrep` добавляет совпадения в исходниках, которые граф не покрыл; у дополнения отдельное продолжение `grepCursor`. Текстовый поиск также видит комментарии, строки, конфиги, шаблоны и скрипты вне графа.

Для `atlas_context` направление `in` даёт дерево вызывающих. Класс при `both` тоже получает дерево использования его членов; метод при `both/out` получает локальный граф. Члены класса не разворачиваются целиком: для их списка используйте `atlas_outline`. `truncated`, `depth_limited`, курсоры и счётчики пропущенных элементов нужно учитывать при оценке полноты ответа.

Фильтр `area` применяется к поиску и grep, а в outline и деревьях вызывающих — к местам использования. Он не ограничивает весь локальный граф `both/out` у метода. `include_tests` управляет поиском и вызывающими; это не запрет чтения тестового файла через `atlas_read`.

Пример аргументов `tools/call`:

```json
{"name":"atlas_search","arguments":{"query":"Wallet::refund","mode":"symbol","kinds":["method"]}}
```

Полученный ID используйте в следующих запросах; пример ID зависит от проекта:

```json
{"name":"atlas_outline","arguments":{"entity_id":"method:App\\Wallet::refund","sections":["calls","called_by"]}}
```

Stdio подключается к той же PostgreSQL и не запускает второй watcher:

```sh
docker compose exec -T atlas code-atlas stdio
```

stdout содержит только JSON-RPC по одной строке на сообщение, логи направляются в stderr. Доступ stdio определяется процессом, который его запускает.

## Доверие и аналитика

Уровень 1 создаёт анализатор, уровень 2 — исправление агента, уровень 3 — подтверждение разработчика. Правка уровня 2 действует сразу, пока ожидает проверки. `approve` повышает доверие до 3; `decline` возвращает предыдущее семантическое состояние. Запрос с устаревшим `expected_revision` получает конфликт.

Агент меняет `metadata` существующего объекта и, у ребра, цель `to`. Создание/удаление, смена ID, типа и позиции через correct запрещены. Анализатор сохраняет защищённые знания при переиндексации, обновляя позиции отдельно. Подтверждённое удаление исходного кода может удалить объект любого уровня. Подробности сопоставления и отката — в [How works.md](How%20works.md).

Веб-аналитика считает графовые элементы, выданные MCP-инструментами `search`, `context` и `outline`, включая их вызовы в batch. `grep` и `read`, просмотр веба и индексация не увеличивают эти счётчики. Один элемент учитывается один раз в выдаче; следующая выдача учитывается отдельно. Метрика показывает выдачу контекста, без доказательства фактического использования агентом. Период — UTC `[from,to)`, идентификационные снимки сохраняются после удаления объекта. Автоматическая очистка истории не настроена.

## Сборка образа

В репозитории Atlas задайте выбранный вами тег:

```sh
export ATLAS_IMAGE=your-registry/code-atlas:your-tag
docker build --pull -t "$ATLAS_IMAGE" .
docker run --rm --entrypoint code-atlas "$ATLAS_IMAGE" plugins
```

После входа в свой реестр образ можно опубликовать командой `docker push "$ATLAS_IMAGE"`. Альтернативные publisher-команды:

```sh
docker compose -f docker-compose.build.yml build atlas
docker compose -f docker-compose.build.yml push atlas
```

Сборка AMD64+ARM64 описана в [docs/distribution.md](docs/distribution.md). Образ содержит скомпилированное приложение, миграции и веб-ресурсы; потребителю исходники Atlas не нужны.

## Локальная разработка

Нужны Rust/Cargo и libpq; для работы сервера — PostgreSQL. [build.rs](build.rs) находит библиотеку libpq; при необходимости задайте `ATLAS_LIBPQ_DIR`. Приложение получает ENV процесса: файл `.env` для `cargo run` автоматически не загружается.

```sh
cargo run --locked -- plugins
cargo run --locked -- migrate
cargo run --locked -- index
cargo run --locked -- serve
```

`plugins` работает без БД и конфигурации проекта. Остальным командам нужны `ATLAS_DATABASE_URL`, `ATLAS_PROJECT_ROOT`, оба токена и доступная БД. `migrate` применяет схему; `index` выполняет один полный проход; `serve` запускает HTTP и watcher. Миграции также применяются при старте `index`, `serve` и `stdio`.

Плагины `php`, `pgsql` и `yii2` зарегистрированы независимо. Новый язык реализует `LanguagePlugin`, фреймворк — `FrameworkPlugin`; регистрация находится в [src/plugins/mod.rs](src/plugins/mod.rs). Новый плагин требует сборки бинарника. Выбор уже встроенных плагинов задаётся в setup-файле. Полноценных анализаторов JavaScript, Go, Rust и Python пока нет; поиск URL в JS/HTML/Vue относится к Yii2-расширению. Контракт и границы SQL — [docs/plugins.md](docs/plugins.md), проверки — [docs/testing-native-pgsql.md](docs/testing-native-pgsql.md).

## Проверки и текущие ограничения

```sh
cargo test --locked
node --test web/model.test.mjs web/indexing.test.mjs web/explore.test.mjs
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
python3 tests/compose_distribution.py
openspec validate --all --strict --no-interactive
```

Проверка этой рабочей копии 10 ноября 2026 года: обычный Rust-набор прошёл, 19 веб-тестов прошли, форматирование и Clippy прошли, 7 Compose-контрактов прошли, каталог `plugins` успешно выведен. Rust-набор включает native runtime с анализом без PHP/Composer. Реальные PostgreSQL-тесты помечены ignored и этим запуском не проверены.

Строгая OpenSpec-валидация: 21 изменение прошло, 2 не прошли — `batch-foreach-and-closures` и `yii2-events` не содержат spec delta. Это состояние спецификационных артефактов; тесты одноимённых возможностей выполнялись в Rust-наборе. Запуск контейнера, сборка/публикация образа и живая проверка браузера в этом проходе не выполнялись.

Реальную PostgreSQL-интеграцию запускайте отдельно на одноразовой БД:

```sh
ATLAS_TEST_DATABASE_URL='host=127.0.0.1 dbname=atlas_test user=atlas' cargo test --locked --test core_postgres -- --ignored --test-threads=1
```

PHP-парсер структурный и ограниченный; это не полный компилятор и не полный межпроцедурный анализ. Динамические имена, runtime-конфигурация и неподдержанный синтаксис могут оставлять пробелы. Источник типов `annotation` требует проверки. Двух проходов вывода возвратов недостаточно для произвольных цепочек и рекурсии. Подробные примеры, диагностики и ограничения — [How works.md](How%20works.md).

В текущем конвейере нет сбора Git blame, истории и diff. Старые отчёты `docs/testing-git.md`, `docs/testing-native-git.md` и исходная спецификация описывают исторические этапы; они не подтверждают наличие Git-функций в текущем приложении.

## Карта репозитория

| Каталог или файл | Ответственность |
| --- | --- |
| `src/domain.rs`, `src/service.rs` | Общая модель графа и прикладные правила. |
| `src/repository.rs`, `src/postgres.rs`, `migrations/` | Контракт хранения, память для тестов, PostgreSQL и схема. |
| `src/config.rs`, `src/setup.rs`, `src/project.rs` | Настройки, области, отбор файлов и обнаружение изменений. |
| `src/indexer.rs`, `src/analyzer/`, `src/parallel.rs` | Конвейер анализа, реестр плагинов, инкрементальная область и потоки. |
| `src/plugins/php/`, `src/plugins/pgsql/`, `src/plugins/yii2/` | PHP, SQL/pgsql (в т.ч. SQL-in-PHP) и дополнение Yii2. |
| `src/http.rs`, `src/mcp_text.rs`, `src/grep_search.rs` | HTTP/MCP, компактная выдача, текстовый поиск и чтение. |
| `src/progress.rs`, `src/diagnostics_log.rs` | Прогресс и диагностические логи. |
| `web/` | Встроенный интерфейс и тесты его моделей. |
| `tests/`, `examples/`, `scripts/` | Поведенческие проверки, примеры, профилирование и агентский benchmark. |
| `Dockerfile`, `docker/`, `docker-compose*.yml` | Сборка образа, совместный lifecycle Atlas/PostgreSQL и установка. |
| `openspec/changes/` | Дизайн, предлагаемые изменения, сценарии, задачи и свидетельства проверок. |
| `code-atlas-specification.md` | Исходное ТЗ; текущую реализацию нужно читать с учётом последующих решений. |

Дополнительно: [контракты плагинов](docs/plugins.md), [архитектура исходного изменения](openspec/changes/add-code-atlas/design.md), [сопоставление с ТЗ](openspec/changes/add-code-atlas/traceability.md), [отчёты проверок](openspec/changes/add-code-atlas/verification.md), [методика агентского benchmark](docs/benchmark-agent.md). Отчёты относятся к указанным в них версиям и условиям запуска.
