# Сборка, публикация и подключение Atlas

Atlas поставляется готовым Docker-образом. Сборка выполняется в репозитории Atlas; проект-получатель использует опубликованный образ через `image:`. Ему нужны только настройки сервиса, read-only доступ к своему коду и отдельный volume для данных атласа.

## Публикация одной платформы

Репозиторий владельца — [drlenux/memory-tools](https://hub.docker.com/repository/docker/drlenux/memory-tools/general). Для текущей версии приложения выбран тег `0.1.0`. Адрес задан, но публикация из текущей среды пока не выполнена.

```sh
export ATLAS_IMAGE=drlenux/memory-tools:0.3.6
docker login
docker compose -f docker-compose.build.yml build atlas
docker run --rm "$ATLAS_IMAGE" plugins
docker compose -f docker-compose.build.yml push atlas
```

Dockerfile собирает только образ приложения (`debian-slim` + бинарник). БД в Compose — официальный `postgres:17-bookworm` (`ATLAS_DB_IMAGE`), без pgvector и без публикации своего db-образа. Пароли, токены и каталог проекта задаются только при запуске Compose.

Имеется в виду публикация конкретного `ATLAS_IMAGE`; дополнительных `latest` команды не создают.

## Обновление образа с прогрессом индексации

Индикатор и API `/api/indexing` добавлены в исходники; их наличие в опубликованном `0.1.0` не подразумевается. Назначьте новый тег, например `0.1.0-progress`, и соберите его в каталоге Atlas:

```sh
ATLAS_IMAGE=drlenux/memory-tools:0.1.0-progress docker compose -f docker-compose.build.yml build atlas
```

Если проект-получатель использует другой Docker daemon, опубликуйте этот тег командой publisher `push atlas` с тем же `ATLAS_IMAGE` и выполните `docker compose pull atlas` у получателя. При локальном образе в том же daemon pull не нужен. Затем у получателя задайте `ATLAS_IMAGE=drlenux/memory-tools:0.1.0-progress` в `.env` и примените:

```sh
docker compose up -d --no-build --force-recreate atlas
docker compose logs -f --tail=50 atlas
```

Существующий volume базы используется повторно. В браузере обновите страницу и подключитесь с reviewer-токеном: панель индексации расположена над вкладками. Она показывает прогресс текущего этапа и его приблизительный ETA; неопределённые этапы и потеря связи обозначаются явно. Строка `state=completed ... 100% committed` в логах появляется после успешной публикации графа.

Этот пример не означает, что тег уже собран или опубликован. В среде агента Docker socket недоступен; проверенная локальная сборка Rust не заменяет сборку Linux-образа.

## Параллельная индексация

Рабочие потоки добавлены в исходники Atlas. Для первой установки этой версии соберите новый образ в каталоге Atlas:

```sh
ATLAS_IMAGE=drlenux/memory-tools:0.1.0-parallel docker compose -f docker-compose.build.yml build atlas
```

Это пример нового тега, а не указание на уже опубликованный образ. Для другого Docker daemon сначала опубликуйте его через publisher `push atlas` с тем же `ATLAS_IMAGE`, затем выполните pull у получателя, как описано выше.

В `.env` проекта-получателя установите:

```dotenv
ATLAS_IMAGE=drlenux/memory-tools:0.1.0-parallel
ATLAS_CPU_LIMIT=4
ATLAS_INDEX_WORKERS=4
```

Оба поставляемых consumer Compose уже передают `ATLAS_INDEX_WORKERS`. Если сервис ранее скопирован в собственный Compose, добавьте в его `environment` строку `ATLAS_INDEX_WORKERS: ${ATLAS_INDEX_WORKERS:-0}` и убедитесь, что `cpus` использует `${ATLAS_CPU_LIMIT:-1}`. Параметр в `.env` сам по себе не передаётся внутрь контейнера.

```sh
docker compose config --quiet
docker compose up -d --no-build --force-recreate atlas
docker compose logs -f --tail=50 atlas
```

При разборе PHP и сборе Git-истории логи показывают `workers=занятые/предел`, а веб-панель — число занятых потоков. Общие графовые проходы и транзакция публикации остаются последовательными. Один PID Atlas теперь может выполнять несколько независимых задач одновременно.

`ATLAS_INDEX_WORKERS=0` выбирает число потоков по округлённому вверх CPU limit в пределах 1..32; явное значение 1 возвращает последовательный режим. Если оставить `ATLAS_CPU_LIMIT=1`, увеличение workers не увеличит доступную CPU quota. `ATLAS_MEMORY_LIMIT_MB` ограничивает весь контейнер вместе с PostgreSQL: общий Git cache не дублируется на каждый поток, но временные буферы могут увеличить расход памяти. Многопоточность не подтверждает устранение прежнего SIGKILL.

Для крупных Yii-проектов (порядок Slotor) при `ATLAS_CPU_LIMIT=1` полный проход занимает порядка 10 минут: поднимите `index.analysis_timeout_secs` (по умолчанию 900) и при OOM/SIGKILL увеличьте `ATLAS_MEMORY_LIMIT_MB` (лимит общий с PostgreSQL). Entrypoint режет `shared_buffers`/`work_mem` Postgres от этого лимита (~1/16 и ~1/64), чтобы анализ не конкурировал с дефолтными буферами PG. Пик RSS снижен: flow батчами, corpus дропается до `php_finalize`, origins не встраивает returns/snippets; full publish (≥256 путей) не грузит весь previous graph — SQL `clear_analyzer_facts` для `trust≤1`, в RAM только protected; `put_nodes`/`put_edges` стримятся чанками по 2000; после analyze и commit — `malloc_trim`. На 2 GiB: без `snippet` в returns, `pgsql` до `php`, spill snapshot на диск перед publish, lean SQL-clear. Лимит поднимать нельзя — пик анализа всё ещё главный риск на очень больших графах.

После установки нового образа изменение CPU/workers требует только пересоздания сервиса; прежний volume базы сохраняется. Локальные замеры и границы проверки: [testing-parallel-indexing](testing-parallel-indexing.md).

## Один тег для AMD64 и ARM64

Обычный build создаёт вариант для платформы builder. Для использования одного тега на Linux AMD64 и ARM64 нужен builder с поддержкой обеих платформ: native workers либо настроенная эмуляция. После настройки выполните в репозитории Atlas:

```sh
docker buildx build --platform linux/amd64,linux/arm64 --tag "$ATLAS_IMAGE" --push .
docker buildx imagetools inspect "$ATLAS_IMAGE"
```

Здесь `--push` сразу публикует manifest list и варианты образа. Поддержку платформ базовыми образами и запуск каждого варианта надо проверить в среде сборки; в текущем sandbox сборка не выполнена. Подготовка builder и выбор стратегии — в [Docker multi-platform builds](https://docs.docker.com/build/building/multi-platform/).

## Подключение в другом проекте

1. Добавьте `services.atlas-db`, `services.atlas` и `volumes.atlas-postgres` из [examples/compose.atlas.yml](../examples/compose.atlas.yml) в Compose проекта. Полный вариант — [docker-compose.yml](../docker-compose.yml).
2. В окружение проекта:

```dotenv
ATLAS_IMAGE=drlenux/memory-tools:0.3.6
ATLAS_DB_IMAGE=postgres:17-bookworm
ATLAS_POSTGRES_PASSWORD=
ATLAS_AGENT_TOKEN=
ATLAS_REVIEW_TOKEN=
```

Заполните пароль и два разных токена (`openssl rand -hex 32`).

3. В каталоге проекта:

```sh
docker compose config --quiet
docker compose pull atlas-db atlas
docker compose up -d --no-build atlas-db atlas
docker compose logs -f atlas
```

Интерфейс и MCP доступны на `http://localhost:8080` и `http://localhost:8080/mcp`. Веб использует reviewer-токен, MCP — agent-токен. Для другого порта задайте `ATLAS_HOST_PORT`; localhost origins подстроятся автоматически. Для reverse proxy явно задайте `ATLAS_ALLOWED_ORIGINS` JSON-массивом.

Исходный проект монтируется в `/project` read-only; по умолчанию это `.` относительно Compose проекта. `ATLAS_HOST_PROJECT_ROOT` позволяет выбрать другой каталог. `create_host_path: false` не позволяет опечатке превратиться в автоматически созданную пустую папку. При нескольких `-f` относительные пути относятся к первому Compose-файлу; это [правило Compose](https://docs.docker.com/compose/how-tos/multiple-compose-files/merge/).

Файлы должны быть доступны на чтение и обход пользователю `postgres` внутри контейнера. Если `.git` linked worktree ссылается на metadata вне примонтированного каталога, для Git-истории потребуется также доступ к этим путям или обычный clone. При недоступной истории Atlas выдаёт диагностику.

## Независимость от приложения

- PostgreSQL Atlas — отдельный сервис `atlas-db`; БД приложения не используется. Пароль/`DB`/`USER` задаются через `ATLAS_POSTGRES_*` и попадают в `POSTGRES_*` у БД и `PG*` у app.
- Volume `atlas-postgres` на `atlas-db` получает префикс Compose-проекта.
- Лимиты CPU/RAM у app и DB раздельные (`ATLAS_MEMORY_LIMIT_MB`, `ATLAS_DB_MEMORY_LIMIT_MB`). Языки/фреймворки — в `.atlas/setup.yaml`.
- Healthcheck app — HTTP (`code-atlas healthcheck`); у БД — `pg_isready`. Готовность графа — по логам/`/api/indexing`, не только по healthy.

## Обновление

Задайте новый тег в `ATLAS_IMAGE` (БД — stock `postgres:17-bookworm`), сохранив Compose-проект, volume и параметры БД:

```sh
docker compose pull atlas-db atlas
docker compose up -d --no-build atlas-db atlas
```

Данные атласа сохраняются в volume. Начальные переменные PostgreSQL применяются при создании БД; смена пароля/пользователя в ENV не меняет уже созданные роли. Для PostgreSQL major upgrade нужен отдельный план миграции данных. Эти ограничения следуют из [контракта PostgreSQL image](https://hub.docker.com/_/postgres).

Для локальной разработки Atlas оба файла можно объединить:

```sh
docker compose -f docker-compose.yml -f docker-compose.build.yml up --build -d atlas
```

## Ошибка 403 при загрузке `/assets/app.js`

Если консоль браузера показывает `GET .../assets/app.js ... 403 (Forbidden)`, обработчик кнопки подключения не загружается и токен ещё не проверяется. В прежнем HTTP guard список API origins применялся также к публичным JS-модулям. При внешнем порту 8085 и origins для 8080 это воспроизводит 403; обычная HTML-форма может перезагружать страницу с `/?`.

Для уже собранного образа задайте в `.env` проекта-получателя:

```dotenv
ATLAS_HOST_PORT=8085
ATLAS_ALLOWED_ORIGINS=["http://localhost:8085","http://127.0.0.1:8085"]
```

В существующем `services.atlas.environment` должна передаваться переменная:

```yaml
ATLAS_ALLOWED_ORIGINS: '${ATLAS_ALLOWED_ORIGINS}'
```

Поставляемые Compose-файлы уже передают её. Наличие записи только в `.env` не передаёт переменную контейнеру автоматически. Пересоздайте сервис с прежним volume и обновите страницу:

```sh
docker compose up -d --no-build --force-recreate atlas
```

Эта настройка не требует пересборки. В исходниках guard также исправлен: публичные HTML/CSS/JS больше не зависят от API origins. Для получения исправленного guard нужен новый образ; API продолжает требовать разрешённый Origin (если заголовок присутствует) и токен соответствующей роли.

## Ошибка `ATLAS_PROJECT_ROOT is required`

Если PostgreSQL запускается, а затем Atlas завершается с этой ошибкой, добавьте в существующий `services.atlas.environment` обязательные настройки приложения:

```yaml
ATLAS_PROJECT_ROOT: /project
ATLAS_BIND: 0.0.0.0:8080
ATLAS_DATABASE_URL: "host=127.0.0.1 connect_timeout=5"
```

Остальные переменные и mount исходников в `/project` оставьте из примера. Минимальный Compose теперь явно передаёт эти значения, чтобы не зависеть от ENV defaults собранного образа. Примените настройки в каталоге проекта:

```sh
docker compose up -d --no-build --force-recreate atlas
docker compose logs --tail=50 -f atlas
```

Пересборка образа для этого исправления не нужна. Пересоздание сервиса сохраняет данные в существующем именованном volume.

## Что проверено здесь

`python3 tests/compose_distribution.py` проверяет Compose в отдельном чужом каталоге без Dockerfile/исходников Atlas, совместно с посторонней БД и конфликтующими POSTGRES_* переменными. Проверяются image-only установка, самостоятельность примера, явная передача обязательных ENV приложения, credentials, путь mount, volume, origins при другом порте, образ по умолчанию/переопределение и обязательность отдельного пароля и сборка без runtime-секретов.

Фактические build, registry push/pull и запуск образа здесь не выполнены: Docker API socket возвращает permission denied. Репозиторий drlenux/memory-tools указан владельцем; тег 0.1.0 выбран по текущей версии приложения. Статическая проверка конфигурации не означает, что образ опубликован или что обе архитектуры прошли запуск.
