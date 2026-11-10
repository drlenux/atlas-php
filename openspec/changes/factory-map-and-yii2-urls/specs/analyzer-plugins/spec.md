# Analyzer plugins — MAP factory

## ADDED Requirements

### Requirement: PLUGIN-20 Class-const MAP keys as read_site

Чтение `Other::CONST` в RHS `private const MAP = […]` SHALL публиковать `read_site` (и `reads` на целевую константу), чтобы поиск по `kinds:["read_site"]` находил строку инициализатора.

#### Scenario: KeeperKey in MAP

- **WHEN** `private const MAP = [KeeperKey::AGGREGATORUA => Foo::class]`
- **THEN** существует `read_site` со span на `KeeperKey::AGGREGATORUA` и `reads` к `constant:…KeeperKey::AGGREGATORUA`

### Requirement: PLUGIN-21 new from MAP class-string values

Значения `Foo::class` в class-const MAP SHALL сохраняться как `class_string_values`; присваивание `$c = self::MAP[$k]` SHALL давать union `class-string<…>`; `new $c` SHALL резолвить конструкторы этих классов.

#### Scenario: Factory forProvider

- **WHEN** MAP содержит `AggregatoruaFreespinProvider::class` и код делает `$class = self::MAP[$a]; return new $class($p);`
- **THEN** есть `calls` к `…AggregatoruaFreespinProvider::__construct` (как candidate среди значений карты)
