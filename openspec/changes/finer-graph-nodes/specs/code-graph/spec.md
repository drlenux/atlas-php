# Code graph — finer nodes

## ADDED Requirements

### Requirement: GRAPH-07 Ветки и места эффекта

Система SHALL представлять условные ветки как вершины `branch` с текстом условия и накопленным путём условий. Система SHALL представлять каждое распознанное место вызова, чтения, записи и возврата как вершину соответствующего kind (`call_site`, `write_site`, `read_site`, `return_site`). Рёбра `calls`, `reads` и `writes` SHALL исходить от occurrence-вершины, а не от method/function. Method/function SHALL быть связан с принадлежащими ему branch и occurrence рёбрами `contains`.

#### Scenario: Условная запись в ветке

- **WHEN** внутри `if ($flag)` выполняется запись свойства
- **THEN** существует вершина `branch` с условием, содержащим `$flag`
- **AND** существует `write_site` с условиями пути, включающими это условие
- **AND** ребро `writes` исходит от `write_site`

#### Scenario: Вызов не привязан напрямую к method как from

- **WHEN** метод `A::foo` вызывает `B::bar`
- **THEN** ребро `calls` имеет `from` вида `call_site:...`
- **AND** method `A::foo` связан с этим `call_site` ребром `contains`

### Requirement: GRAPH-08 Признак закомментированности

Каждая вершина графа кода SHALL иметь булев признак `commented` в metadata. Для кода вне комментариев значение SHALL быть `false`. Для сущностей, извлечённых только из текста комментария, значение SHALL быть `true`.

#### Scenario: Активный метод

- **WHEN** метод объявлен в исполняемом PHP вне комментария
- **THEN** у его вершины `commented` равно `false`

#### Scenario: Закомментированный вызов

- **WHEN** в комментарии внутри метода встречается PHP-подобный вызов `$service->run()`
- **THEN** в графе есть occurrence с `commented` равным `true`
- **AND** выражение occurrence отражает этот вызов

### Requirement: GRAPH-09 Лёгкие method и function

Нода метода или функции SHALL NOT включать полный исходный текст тела в полях `body` или `source`. Расположение задаётся span; фрагменты кода живут на occurrence/`return_site` или читаются из файла по span.

#### Scenario: Method без тела в metadata

- **WHEN** индексируется метод с непустым телом
- **THEN** metadata метода не содержит ключей `body` и `source`
- **AND** span метода указывает на его расположение в файле
