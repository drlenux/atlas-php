# Tasks: finer-graph-nodes

## 1. Контракт и тесты (TDD)

- [x] 1.1 Тест: method/function без `body`/`source`, с `commented: false`
- [x] 1.2 Тест: `branch` + `call_site`/`write_site`; `calls`/`writes` from occurrence; `contains` от method
- [x] 1.3 Тест: `// $x->run()` → occurrence с `commented: true`
- [x] 1.4 Обновить существующие native_php / native_yii2 сценарии под новый from у рёбер

## 2. PHP-анализатор

- [x] 2.1 Лексер сохраняет comment spans
- [x] 2.2 Method metadata без body/source; все ноды с `commented`
- [x] 2.3 Эмит `branch` в flow для if/elseif/else
- [x] 2.4 Эмит occurrence-нод и перенос from рёбер calls/reads/writes
- [x] 2.5 `return_site` + contains
- [x] 2.6 CommentIndex best-effort

## 3. Совместимость

- [x] 3.1 Yii2-плагин: тело метода из span файла
- [x] 3.2 Context BFS: occurrence отдаёт один hop эффекта; search не шумит по site id
- [x] 3.3 `cargo test --locked`, clippy `-D warnings`, OpenSpec validate

## 4. Валидация OpenSpec

- [x] 4.1 `openspec validate finer-graph-nodes --strict`
