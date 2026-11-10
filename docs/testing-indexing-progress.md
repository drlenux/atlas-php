# Прогресс индексации: проверка 2026-11-04

Архитектура и контракт определены до production-кода в [design.md](../openspec/changes/add-code-atlas/design.md#прогресс-индексации-2026-11-04), сценарии — INDEX-05. Изменение добавляет наблюдаемость; оно не исправляет и не подтверждает причину предыдущего SIGKILL/OOM.

## Red → green

- HTTP: новый авторизованный GET `/api/indexing` возвращал **404 вместо 200**. После добавления read model API отвечает, даже когда отдельный поток удерживает AtlasService; пустой токен и agent token получают 401, cache-control — no-store.
- Индексатор: новые тесты не компилировались из-за отсутствия `indexing_progress`, `with_progress` и `AnalysisOptions.progress`. После реализации проходят реальная подготовка PHP→Git→публикация, ошибка лимита входа, failure плагина, изменение исходника перед commit и новый run_id при повторной попытке. Подготовленный, но ещё не опубликованный граф остаётся running.
- ETA: сначала отсутствовали модель и формула. Проверены порог ≥3 единиц/≥1 секунды, неизвестный total, нулевой total, сброс счётчика/пути/ETA при смене этапа; время failed/completed замораживается.
- Веб-модель: импорт `indexingView` сначала отсутствовал. Теперь тесты проверяют процент этапа, явную подпись ETA этапа, completed, failed, waiting и отсутствие устаревшего ETA при потере связи.
- Опрос: сначала отсутствовал `indexing.mjs`. Детерминированные тесты с управляемым scheduler проверяют отсутствие перекрывающихся запросов, период сервера, восстановление после ошибки, abort по timeout, игнорирование ответа старого подключения и остановку при 401.
- Логи: отдельная red-фаза обнаружила отсутствие явного `[failed]` и успешного `100% committed`. После исправления ошибка после обработки всех единиц не показывается как успех; 100% всей индексации выводится после commit. Ещё один failing test показал, что ожидание 10% слишком долго для больших этапов: добавлен промежуточный отчёт при следующем обновлении после пяти секунд, даже до первых 10%; частые события по-прежнему ограничены.

## Выполненные проверки

- `cargo test --offline --locked`: 94 Rust-теста проходят; 3 теста настоящего PostgreSQL явно ignored без disposable database.
- `node --test web/model.test.mjs web/indexing.test.mjs`: 9 проходят, включая 4 прежние проверки dashboard.
- `node --check web/app.js`: пройдено.
- `cargo clippy --offline --locked --all-targets -- -D warnings`, `cargo fmt --all -- --check`: пройдено. Удержание тестовой блокировки вынесено в отдельный поток; Clippy не подавлялся.
- `cargo build --offline --locked --release`: локальная сборка Rust с встроенными web assets.
- `openspec validate --all --strict --no-interactive`: пройдено.

## Границы проверки

Живой браузер недоступен: browser tool вернул `The browser access host is unavailable`; проба локального TCP listener завершилась `listen EPERM: operation not permitted 127.0.0.1`. Скриншот и browser e2e не получены. Проверка HTTP выполняется in-process, проверка UI — на моделях и poller, без заявления о проверенном рендеринге.

Docker socket недоступен в этой сессии. Образ с новыми исходниками не собран, не опубликован и не запущен на проекте владельца. Linux/ARM64 и PostgreSQL runtime остаются отдельной проверкой. Миграции и существующие данные не изменялись. Для доставки нужен новый образ: [инструкция](distribution.md#обновление-образа-с-прогрессом-индексации).

## incremental-indexing

Change: `openspec/changes/incremental-indexing`.

1. `tests/incremental.rs` (поверхность объявлений и область): 4 из 5 упали на заглушке, затем прошли.
2. `tests/incremental_indexing.rs` (координатор на настоящих файлах): не компилировались без `index_changes`/`last_scope`. После реализации упали 2 из 5. Оба падения — ошибки в моих тестах: имя `return_site` — это выражение, а переименование по замыслу сохраняет id узла. Тесты исправлены, а не код.
3. `pending_paths` (`tests/project.rs`, `tests/agent_navigation.rs`): red на компиляции, затем зелёные.
4. На сервере владельца инкрементальная сборка упала с «snapshot edge refers to an unknown node». Воспроизведено двумя тестами (`edit_of_a_caller_publishes_edges_into_unchanged_files`, `partial_publication_accepts_edges_to_stored_nodes_of_other_paths`) с той же ошибкой. Исправлено, добавлен PostgreSQL-тест `existing_node_ids`.

Не сделано: замер задержки «правка → поиск» на живом сервере.

```sh
cargo test --locked    # 182 passed, 8 ignored
ATLAS_TEST_DATABASE_URL=… cargo test --locked --test core_postgres -- --ignored --test-threads=1   # 7 passed
```
