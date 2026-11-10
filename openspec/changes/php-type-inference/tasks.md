# Tasks: php-type-inference

## 1. PHPDoc

- [x] 1.1 Тесты `doc.rs`: теги, нормализация типов, `$this`, обобщения, массивы
- [x] 1.2 `doc.rs`, привязка docblock к объявлениям в `syntax.rs`

## 2. Вывод типов

- [x] 2.1 Тесты сценариев PLUGIN-08/09 (`tests/native_php_types.rs`)
- [x] 2.2 `Typed`, `refine`, объединения/массивы/обобщения, `static`/`@template`
- [x] 2.3 Локальные `@var`, `foreach`, `@property`-вершины, `resolution: annotated`

## 3. Vendor-объявления

- [x] 3.1 Тесты: ENV-валидация, discovery, отсутствие flow-вершин в vendor
- [x] 3.2 `ATLAS_DECLARATION_PATHS`, discovery, `AnalysisOptions.declaration_paths`

## 4. Проверка

- [x] 4.1 `cargo test`, fmt, clippy, OpenSpec validate
- [x] 4.2 Замер на `slotor777`: доля распознанных вызовов `processingGameTransaction`, размер индекса, время

## 5. Найдено при замере

- [x] 5.1 Индекс классов: регистронезависимый поиск и наследники за O(1). Без него анализ `slotor777` превышал лимит 120 с, потому что получателей с известным типом стало много
- [x] 5.2 Тест и реализация: `available_via` не создаётся к методам из `ATLAS_DECLARATION_PATHS` (было +148 тыс. рёбер)
- [x] 5.3 Тест и реализация: нераспознанный вызов называет неиндексированного предка (`unindexed_ancestor`), не приписывая причину (PLUGIN-07)
- [x] 5.4 Тест и реализация: `@method` класса как магический метод (`type_source: annotation`); команды `yii\redis\Connection` (211 тегов `@method`)
