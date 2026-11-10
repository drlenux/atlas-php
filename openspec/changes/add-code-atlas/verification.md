# Проверка реализации v0.5

## Удаление Git-provenance (2026-11-05)

По запросу владельца убраны модуль `git`/`git_native`, стадия индексации, `metadata.git`, watcher `<git-head>`, zlib в образе, UI-этап Git и требования GIT-*. На probe-проекте после пересборки `drlenux/memory-tools:probe-local`: ответ `atlas_context` **~137 KiB** вместо ~294 KiB, `git_fields=0`, в логах нет `stage=git/history`. `cargo test`, Clippy, Node, strict OpenSpec пройдены. Каталог `.git` по-прежнему не индексируется как исходный код.

## Исправление разбора PHP-выражений (PLUGIN-05–PLUGIN-07)

2026-11-04 владелец прислал контекст с каскадом `analysis_limitation` и потерей вызова при `(int)Yii::$app->request->post('userId')`. Диагностическое воспроизведение до исправления:

| Выражение | До исправления |
| --- | --- |
| `(int)Yii::$app->request->post('userId')` | 0 calls, 7 `analysis_limitation` |
| `$ok ? 'yes' : 'no'` / `$value ?: []` | 2–3 limitation на операторы |
| `$data['id']` / `(object)[...]` | limitation и потеря структуры |

После TDD (red на постоянных регрессиях в `tests/native_php.rs`, затем `Cast`/`Ternary`/`Index`/`CompoundAssign`/ключи массива в `expression.rs` и обход в `flow.rs`):

- `cast_preserves_call_chain_and_result_type` — вызов `post` сохраняется; тип `(int)/(bool)/(string)`; нет каскада по `::`/`->`.
- `ternary_and_elvis_preserve_branch_calls_without_operator_noise` — ветви с условиями; левый операнд `?:` один раз; `??` без шума.
- `array_index_and_compound_assignment_preserve_calls_and_uses` — чтение `$data['id']`, два `deleteAll`, запись `+=`.
- `nested_object_cast_array_keeps_keys_conditions_and_nested_call` — тип `object`, nested call/nullsafe.
- `cast_index_keeps_calls_while_unresolved_targets_stay_distinct_from_parse_defects` — `unresolved_*` не подменяется parse-defect.
- `expression_graph_identical_for_one_and_four_workers` — одинаковый граф при 1 и 4 workers.
- Прежние Yii2-тесты на vendor getter/setter и унаследованные методы сохранены; syntax error по-прежнему инвалидирует файл.

Локально: **полный `cargo test --locked`**, Clippy `-D warnings`, rustfmt, release, **10 Node**, **7 Compose**, strict OpenSpec.

Живой прогон на тестовом проекте `examples/probe-project` (2026-11-05):

- `cargo run --example probe_verify -- examples/probe-project` и `--vendor`: **ok**, ≥3 `post`, 2 `deleteAll`, ветви `loadA`/`loadB`, 0 limitation-каскада.
- Собран образ `drlenux/memory-tools:probe-local` через `docker-compose.build.yml`.
- Compose в `examples/probe-project` поднял контейнер на `127.0.0.1:18085`; индексация **completed** (`indexed: 30 nodes, 83 edges, 6 diagnostics`).
- MCP `atlas_context` по `method:admin\controllers\TestController::actionProbe`: 3 unresolved `post` (ожидаемо без полной Yii runtime-цепи `$app`), resolved `deleteAll`/`loadA`/`loadB`/`fallback`, **0** `analysis_limitation`.
- `GET /` и `/assets/app.js` с Origin внешнего порта — **200**; `/api/indexing` — completed; `/health` — ok.

Проект владельца вне workspace не монтировался; для него достаточно того же образа и mount своего корня.

## Ограниченные рабочие потоки

По запросу владельца добавлены реальные scoped threads для независимого разбора PHP-файлов и Git-истории. `ATLAS_INDEX_WORKERS=0` выбирает число потоков по `ceil(ATLAS_CPU_LIMIT)` в пределах 1..32, явное значение 1 возвращает последовательный режим. Оба consumer Compose передают ENV. Общие семантические проходы и транзакция публикации остаются последовательными; частичный результат worker не публикуется. Git readers разделяют ограниченный cache и один HEAD. API/UI/logs показывают число занятых потоков.

Полный текущий проход: **104 Rust + 10 Node + 7 Compose**, 3 настоящих PostgreSQL-теста ignored. Clippy, rustfmt, release binary/profile example и strict OpenSpec пройдены. На отдельном fixture из 128 PHP-файлов и шести commits: 1 worker — 5,63–5,68 с / 522–523 MiB peak RSS; 4 workers — 2,65–2,72 с / 514–521 MiB. Это macOS release + MemoryRepository без container quota и PostgreSQL; результат не экстраполируется на проект владельца. Последовательность TDD, параметры замера и ограничения: [testing-parallel-indexing](../../../docs/testing-parallel-indexing.md).

Новый образ не собран и не установлен в контейнер владельца из-за недоступного Docker socket; живой браузер не проверен. Инструкция установки: [параллельная индексация](../../../docs/distribution.md#параллельная-индексация). Предыдущий SIGKILL не выдаётся за устранённый.

## 403 публичного JavaScript при подключении на порту 8085

По сообщению браузерной консоли владельца `GET http://localhost:8085/assets/app.js ... 403` проверен общий HTTP guard. Origin проверялся до выбора защищённого маршрута и применялся к публичным ресурсам. Новый регрессионный тест воспроизвёл 403 вместо 200 при Origin внешнего порта, отсутствующем в списке API origins; соседний тест подтвердил существующие 403/401 для API.

После исправления Origin-проверка применяется к `/api/*` и `/mcp`, а поставляемые HTML/CSS/JS загружаются без токена независимо от API origins. Контракт и решение записаны до кода в RUNTIME-07 и design.md. HTTP-тесты проверяют все четыре asset-зависимости и HTML, MIME/no-store, отсутствие новых CORS-разрешений, роли токенов и сохранённую Origin-проверку API. Инструкция применения настройки к существующему образу: [403 JS](../../../docs/distribution.md#ошибка-403-при-загрузке-assetsappjs).

Живое исправление на `localhost:8085` не подтверждено: curl из среды агента не подключается к этому listener, browser access host недоступен, Orca CLI не запускается из установленной ссылки. Эти ограничения не подменяются in-process HTTP-тестами. Токен пользователя не записывался в исходники или документацию.

Для этого исправления прошли **9 HTTP-тестов** (включая две новые регрессии), **6 Compose-проверок**, Clippy для всех targets, rustfmt, release-сборка и strict OpenSpec. Полный прежний набор Rust/Node повторно не выдаётся за проверку текущего HTTP-изменения.

## Прогресс индексации

По запросу владельца добавлены независимый от транзакции status API `/api/indexing`, индикатор текущего этапа в вебе и текстовая шкала в runtime stderr. Счётчики обновляются при чтении исходников, разборе PHP/анализе функций и сборе истории Git. Оставшееся время оценивается только для текущего этапа при достаточном числе наблюдений; неизвестный объём и ошибки не показываются как успешное завершение.

Проверки текущего изменения: **94 Rust + 9 Node**, 3 настоящих PostgreSQL-теста ignored; Clippy, rustfmt, release build и strict OpenSpec пройдены. TDD и точные ограничения: [testing-indexing-progress](../../../docs/testing-indexing-progress.md). Живой браузер, Docker build/run/push и работа с реальной БД не подтверждены. Эта функция требует пересборки образа и не является исправлением предыдущего SIGKILL.

## Исправление ENV минимального Compose

В логе пользователя от 2026-11-04 PostgreSQL 17.11 принимает соединения, расширение vector уже существует; затем Atlas завершает работу с `ATLAS_PROJECT_ROOT is required`. Этот лог подтверждает запуск БД, но не успешный запуск приложения. Фактические ENV и image контейнера агентом не инспектировались.

В `examples/compose.atlas.yml` явно добавлены ATLAS_PROJECT_ROOT, ATLAS_BIND и ATLAS_DATABASE_URL; раньше пример зависел от defaults образа. В проверку consumer-конфигурации добавлены утверждения для всех трёх параметров. Все **6 Compose-проверок прошли**. Rust и образ не менялись; для уже собранного образа достаточно применить ENV пересозданием сервиса с прежним volume. Успешный запуск приложения после этого исправления пока не подтверждён.

## Поставка Docker-образа v0.5

Владелец указал Docker Hub `drlenux/memory-tools`; для Cargo 0.1.0 подготовлен тег `0.1.0`. Основной Compose и самостоятельный examples/compose.atlas.yml используют image без build; отдельный docker-compose.build.yml предназначен издателю. Image содержит healthcheck и базовые ENV, а host credentials встроенной БД имеют префикс ATLAS_POSTGRES_. Код Rust и плагины в этом изменении не менялись.

- `python3 tests/compose_distribution.py`: **6 прошли**. Проверены установка/merge в чужом каталоге без Atlas checkout, самостоятельность скопированного примера, namespace пароля, согласованные PG/POSTGRES overrides, origin при другом порте, volume/mount, default/override image и publisher build без runtime credentials.
- `cargo test --offline --locked --test native_runtime`: **3 прошли** в отдельном Cargo target worktree поставки.
- Строгий OpenSpec и `git diff --check`: пройдены.
- Наблюдаемая красная фаза: прежний Compose содержал build и подхватывал POSTGRES_* чужой БД; самостоятельный пример/build-рецепт отсутствовали. После исправления 5 первоначальных проверок прошли. После уточнения Docker Hub добавлены default-image и DB-overrides сценарии; все 6 прошли.
- Фактическая команда `docker build --pull -t drlenux/memory-tools:0.1.0 .` завершилась `permission denied while trying to connect to the docker API at unix:///Users/eselbelokudrenko/.orbstack/run/docker.sock`. **В среде агента образ не собран и не опубликован**. Push не запускался после неуспешной сборки; список существующих тегов через веб-инструмент также недоступен.
- Multiarch-команды подготовлены, но manifest и запуск AMD64/ARM64 вариантов не проверены. Подробности: [поставка](../../../docs/distribution.md).

Ниже сохранены результаты полного прохода v0.4 для неизменённого ядра; 88 тестов не выдаются за повторный полный прогон в packaging-изменении.

## Исправленная архитектура

По уточнению владельца удалена ошибочная реализация плагина на PHP. Atlas содержит два нативных Rust-расширения: языковой `src/plugins/php/` и отдельный фреймворковый `src/plugins/yii2/`. Общие LanguagePlugin/FrameworkPlugin и реестр находятся в `src/analyzer/`. Языковой runtime, Composer и Git CLI приложению не требуются; Git читается нативно через объектное хранилище и libz. В Docker runtime добавляются бинарник Atlas и необходимые системные библиотеки к PostgreSQL/pgvector.

Работа продолжена тремя субагентами в отдельных Git worktrees PHP, Yii2 и Git, сведена в `fix/native-analyzers`. Архитектура зафиксирована до кода в [native-analyzer-design](../../../docs/native-analyzer-design.md). История прежней реализации сохраняется в Git bundle; прежние 153 PHP assertions не считаются проверками новой версии.

## Полный проход ядра v0.4

| Проверка | Результат |
| --- | --- |
| `cargo test --offline --locked` | **88 прошли**, 3 настоящих PostgreSQL-теста явно пропущены. PHP/core tests больше не требуют PHP и не помечены ignored. |
| `cargo clippy --offline --locked --all-targets -- -D warnings` | Пройдено без предупреждений. |
| `cargo fmt --all -- --check` | Пройдено. |
| `cargo build --offline --locked --release` | Релизный бинарник собран; `--help` и `plugins` выполнены. |
| `node --test web/model.test.mjs` | **4 прошли**. |
| `docker compose config --quiet`, `bash -n docker/entrypoint.sh` | Пройдено с тестовыми ENV. Это статическая проверка конфигурации, не запуск контейнера. |
| `openspec validate --all --strict --no-interactive` | Изменение add-code-atlas валидно. |

Rust-проверки включают: 3 unit native Git, 5 Config, 2 native PHP→core, 16 Service Layer, 14 Git provenance, 6 HTTP/MCP, 4 indexing, 14 PHP, 3 native runtime, 10 Yii2, 1 corpus resilience, 4 registry, 5 project/watcher и 1 сквозной workflow.

### Что доказано интеграцией

- Реестр маршрутизирует файлы по языкам, отдельно включает фреймворк, отклоняет неизвестные плагины/дубликаты/отсутствующую зависимость и соблюдает бюджет.
- Анализ с пустым PATH проходит настоящий PHP → Yii2 → индексатор → сервис. Выключение Yii2 через конфигурацию оставляет базовый PHP-граф. PHP и Git действительно недоступны дочернему процессу.
- Проверены vendor getter/setter, behaviors, статические компоненты, неоднозначная/динамическая конфигурация и отсутствие Yii-семантики у обычных PHP-классов.
- Ошибка разбора сохраняет подтверждённый объект trust 3; восстановление кода обновляет позиции и сохраняет правку. Реальное удаление остаётся отдельным действием анализатора.
- Сквозной нативный workflow проходит MCP-контекст, агентскую правку, переиндексацию, approve и счётчик выдач. В нём используется MemoryRepository; он не доказывает работу PostgreSQL.
- Нативный Git проверен на loose objects, реальных OFS/REF pack deltas, packed refs и linked worktree с пустым PATH. Все семь прежних Git-регрессий сохранены.

Наблюдаемые red/green и перенос сценариев: [PHP](../../../docs/testing-native-php.md), [Yii2](../../../docs/native-yii2.md), [Git](../../../docs/testing-native-git.md), [реестр/runtime](../../../docs/testing-native-runtime.md).

## Практические границы

PHP-плагин — ограниченный структурный парсер и анализ потока, **не полный компилятор PHP**. Grouped imports, anonymous classes и альтернативный control-flow syntax явно неподдержаны; прочие неполные семантики, включая top-level execution, constants/enum details, attributes и promoted/multiple properties, перечислены в [отчёте PHP](../../../docs/testing-native-php.md). Успех разбора не доказывает, что PHP скомпилировал бы файл. Версионные проверки охватывают документированные конструкции, не всю грамматику выбранной версии.

Yii2 не исполняет bootstrap, DI или вычисляемые конфигурации. Динамические определения остаются кандидатами/неизвестными. Нативный Git ограничен SHA-1 и документированными форматами; неоднозначные merge/изменённые rename, недоступные/shallow объекты и превышение лимитов дают явную недоступность.

JS/Go/Rust/Python пока не реализованы: готов общий контракт добавления языков. Плагины встроены при сборке, ENV выбирает уже зарегистрированные; динамический ABI не обещается. Полная переиндексация области сохраняется; производительность на больших репозиториях не измерена. Аналитика измеряет выдачу графа в MCP, не фактическое использование в решении агента.

## Недоступные проверки среды

1. **PostgreSQL runtime:** предыдущая попытка запуска установленного PostgreSQL 17.11 завершилась `shmget ... Operation not permitted`, включая повтор с mmap-настройками. Три `tests/core_postgres.rs` компилируются, но не выполнены; требуется одноразовая БД с ATLAS_TEST_DATABASE_URL.
2. **Docker build/up/lifecycle:** Docker API socket недоступен (`permission denied`). Новая конфигурация проверена статически; сборка Linux-образа, запуск/остановка PostgreSQL и Atlas не подтверждены.
3. **Browser/TCP:** локальный listener запрещён (`Operation not permitted`); HTTP проверен in-process, браузерный прогон не выполнен. Команда healthcheck проверена компиляцией/конфигурацией, но не живым TCP endpoint.
4. **Git workspace:** создание корневой .git запрещено. Настоящие worktrees используются во временном разрешённом каталоге; исходники доставляются в проект, история всех веток сохраняется в artifacts/code-atlas.git.bundle. 1Password signing socket недоступен; локальные коммиты сделаны с per-command отключением подписи, глобальные настройки не менялись.

Изменение не архивируется как полностью проверенное развёртывание. Недоступные проверки остаются открытыми в tasks.md.
