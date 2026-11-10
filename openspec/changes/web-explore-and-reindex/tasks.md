# Tasks: web-explore-and-reindex

## 1. Переиндексация

- [x] 1.1 Тест: `IndexingProgress` — запрос идемпотентен, `take` сбрасывает, snapshot сообщает `reindex_queued`
- [x] 1.2 Тест: `POST /api/reindex` только с токеном проверяющего, 202, отражается в `/api/indexing`
- [x] 1.3 Реализация в `progress.rs`, `http.rs`, watcher в `main.rs`
- [x] 1.4 Тест модели: состояние кнопки в `indexingView`

## 2. Веб-API

- [x] 2.1 Тест: `/api/search` с `kinds`, `cursor`, `next_cursor`, компактные вершины
- [x] 2.2 Тест: `/api/context` принимает параметры, значения по умолчанию прежние
- [x] 2.3 Реализация

## 3. Интерфейс

- [x] 3.1 Тесты модели: `ownerOf`, `entityLabel`, `contextView`
- [x] 3.2 Реализация `model.mjs`, `app.js`, `index.html`, стили
- [x] 3.3 Иконка без 404
- [x] 3.4 Проверка в браузере на `slotor777` (скриншоты desktop/mobile, консоль без ошибок, кнопка запускает прогон)

## 4. Проверка

- [x] 4.1 `cargo test --locked`, fmt, clippy, `node --test`, `openspec validate --strict`
