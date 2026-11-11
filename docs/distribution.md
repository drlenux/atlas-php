# Сборка, публикация и подключение Atlas

Atlas поставляется готовым Docker-образом. Сборка выполняется в репозитории Atlas; проект-получатель использует опубликованный образ через `image:`. Ему нужны только настройки сервиса, read-only доступ к своему коду и отдельный volume для данных атласа.

## Публикация одной платформы

Репозиторий владельца — [drlenux/memory-tools](https://hub.docker.com/repository/docker/drlenux/memory-tools/general). Для текущей версии приложения выбран тег `0.1.0`. Адрес задан, но публикация из текущей среды пока не выполнена.

```sh
export ATLAS_IMAGE=drlenux/memory-tools:0.1.0
docker login
docker build --pull -t "$ATLAS_IMAGE" .
docker run --rm --entrypoint code-atlas "$ATLAS_IMAGE" plugins
docker push "$ATLAS_IMAGE"
```

Dockerfile компилирует Linux-бинарник в build stage и переносит его в образ PostgreSQL/pgvector вместе с entrypoint и миграцией расширения. Web assets и Rust-плагины встроены в бинарник. Пароли, токены и каталог исследуемого проекта задаются только при запуске.

Для сборки и push также есть отдельный Compose-рецепт без runtime-секретов:

```sh
docker compose -f docker-compose.build.yml build atlas
docker compose -f docker-compose.build.yml push atlas
```

Имеется в виду публикация конкретного `ATLAS_IMAGE`; дополнительных `latest` или других тегов команды не создают. Механизм авторизации и push описан в [официальной документации Docker](https://docs.docker.com/reference/cli/docker/image/push/).

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

Для крупных Yii-проектов (порядок Slotor) при `ATLAS_CPU_LIMIT=1` полный проход занимает порядка 10 минут: поднимите `index.analysis_timeout_secs` (по умолчанию 900) и при OOM/SIGKILL увеличьте `ATLAS_MEMORY_LIMIT_MB` (лимит общий с PostgreSQL). Пик RSS снижен тем, что результаты первого pass flow не удерживаются на время второго, а `php_finalize` не клонирует весь граф.

После установки нового образа изменение CPU/workers требует только пересоздания сервиса; прежний volume базы сохраняется. Локальные замеры и границы проверки: [testing-parallel-indexing](testing-parallel-indexing.md).

## Один тег для AMD64 и ARM64

Обычный build создаёт вариант для платформы builder. Для использования одного тега на Linux AMD64 и ARM64 нужен builder с поддержкой обеих платформ: native workers либо настроенная эмуляция. После настройки выполните в репозитории Atlas:

```sh
docker buildx build --platform linux/amd64,linux/arm64 --tag "$ATLAS_IMAGE" --push .
docker buildx imagetools inspect "$ATLAS_IMAGE"
```

Здесь `--push` сразу публикует manifest list и варианты образа. Поддержку платформ базовыми образами и запуск каждого варианта надо проверить в среде сборки; в текущем sandbox сборка не выполнена. Подготовка builder и выбор стратегии — в [Docker multi-platform builds](https://docs.docker.com/build/building/multi-platform/).

## Подключение в другом проекте

1. Добавьте `services.atlas` и `volumes.atlas-postgres` из [examples/compose.atlas.yml](../examples/compose.atlas.yml) в существующий Compose проекта. Фрагмент самодостаточен и не ссылается на файлы Atlas. Полный вариант всех ENV — [docker-compose.yml](../docker-compose.yml).
2. Добавьте в окружение проекта пароль и два разных токена; ATLAS_IMAGE можно переопределить для другой версии:

```dotenv
ATLAS_IMAGE=drlenux/memory-tools:0.1.0
ATLAS_POSTGRES_PASSWORD=
ATLAS_AGENT_TOKEN=
ATLAS_REVIEW_TOKEN=
```

Заполните пароль и два разных токена. Случайное значение можно получить командой `openssl rand -hex 32`. Для private registry выполните `docker login` на машине-получателе. Вместо тега можно указать `registry/repository@sha256:<digest>`.

3. В каталоге чужого проекта запустите:

```sh
docker compose config --quiet
docker compose pull atlas
docker compose up -d --no-build atlas
docker compose logs -f atlas
```

Интерфейс и MCP доступны на `http://localhost:8080` и `http://localhost:8080/mcp`. Веб использует reviewer-токен, MCP — agent-токен. Для другого порта задайте `ATLAS_HOST_PORT`; localhost origins подстроятся автоматически. Для reverse proxy явно задайте `ATLAS_ALLOWED_ORIGINS` JSON-массивом.

Исходный проект монтируется в `/project` read-only; по умолчанию это `.` относительно Compose проекта. `ATLAS_HOST_PROJECT_ROOT` позволяет выбрать другой каталог. `create_host_path: false` не позволяет опечатке превратиться в автоматически созданную пустую папку. При нескольких `-f` относительные пути относятся к первому Compose-файлу; это [правило Compose](https://docs.docker.com/compose/how-tos/multiple-compose-files/merge/).

Файлы должны быть доступны на чтение и обход пользователю `postgres` внутри контейнера. Если `.git` linked worktree ссылается на metadata вне примонтированного каталога, для Git-истории потребуется также доступ к этим путям или обычный clone. При недоступной истории Atlas выдаёт диагностику.

## Независимость от приложения

- PostgreSQL Atlas находится внутри его контейнера; БД приложения не используется. Хостовые `ATLAS_POSTGRES_DB`, `ATLAS_POSTGRES_USER`, `ATLAS_POSTGRES_PASSWORD` явно отображаются в соответствующие `POSTGRES_*` и `PG*` внутри Atlas.
- Volume `atlas-postgres` получает префикс Compose-проекта. Для разных проектов используются разные данные, без глобального имени volume или container_name.
- CPU/RAM ограничиваются Compose-параметрами, которые согласованы с ENV Atlas. Список языков и фреймворков, vendor и остальные параметры настраиваются ENV; дополнительной установки плагинов внутри контейнера не требуется.
- Healthcheck находится в образе и проверяет PostgreSQL плюс HTTP; завершение начальной индексации видно по логам, а не по одному статусу healthy.

## Обновление

Задайте новый тег или digest в `ATLAS_IMAGE`, сохранив имя Compose-проекта, volume и параметры существующей БД:

```sh
docker compose pull atlas
docker compose up -d --no-build atlas
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
