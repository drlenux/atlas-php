# Project indexing — объявления сторонних библиотек

## ADDED Requirements

### Requirement: INDEX-08 Индекс объявлений сторонних библиотек

Система SHALL индексировать объявления (классы, методы, свойства, аннотации, наследование) из путей `ATLAS_DECLARATION_PATHS`, даже если vendor исключён, и SHALL NOT анализировать тела функций в этих путях. По умолчанию `ATLAS_DECLARATION_PATHS` равен `["vendor/yiisoft"]`. Несуществующий путь SHALL пропускаться. Абсолютный путь или путь с `..` SHALL останавливать запуск с объяснением.

#### Scenario: Наследник ActiveRecord

- **WHEN** класс проекта наследует `yii\db\ActiveRecord`, а `vendor/yiisoft/yii2/db/ActiveRecord.php` существует
- **THEN** ребро `inherits` ведёт к вершине `class:yii\db\ActiveRecord`
- **AND** диагностики `Definition not indexed: yii\db\ActiveRecord` нет
- **AND** в vendor-файлах нет вершин `call_site`, `read_site` и `variable`

#### Scenario: Недопустимый путь

- **WHEN** `ATLAS_DECLARATION_PATHS` содержит `../other`
- **THEN** запуск останавливается с сообщением о недопустимом пути
