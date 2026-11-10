# Tasks: yii2-framework-members

## 1. Контракт

- [x] 1.1 Тесты: сценарии PLUGIN-10 через реестр (`tests/yii2_members.rs`)
- [x] 1.2 `MemberSource`, `LanguagePlugin::analyze_with_members`, `FrameworkPlugin::members`, `FrameworkMembers` в реестре

## 2. PHP-плагин

- [x] 2.1 Слияние членов: приоритет код > фреймворк > аннотация; `contains`, `implemented_by`
- [x] 2.2 `flow.rs`: аннотацией считается только `type_source: annotation`; рёбра через члены фреймворка получают `framework`

## 3. Yii2

- [x] 3.1 Связи `hasOne`/`hasMany`
- [x] 3.2 Компоненты приложения

## 4. Проверка

- [x] 4.1 `cargo test`, fmt, clippy, OpenSpec validate
- [x] 4.2 Замер на `slotor777`

## 5. Найдено при замере

- [x] 5.1 Тест и реализация: запись компонента без `class` только настраивает компонент (Yii сливает её с записью, где `class` задан)
- [x] 5.2 Тест и реализация: диагностика `dynamic_component_map` с файлом и строкой. На `slotor777` это `scripts/qa-stand/config/common-main.php`, где `...$queues` делал неустановленной всю карту компонентов
- [x] 5.3 `Index::new` в Yii2 искал вершины линейно для каждого ребра `available_via`; заменено на словарь по id
