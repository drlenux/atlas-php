# Analyzer plugins — вывод типов из аннотаций

## ADDED Requirements

### Requirement: PLUGIN-08 Типы из PHPDoc с отметкой происхождения

PHP-анализатор SHALL использовать типы и члены из PHPDoc (`@var`, `@param`, `@return`, `@property`, `@property-read`, `@property-write`, `@method`, `@template`), если код не даёт информативного типа или аннотация уточняет тот же класс. Каждая связь, найденная через тип, зависящий от аннотации, SHALL иметь `resolution` `annotated` и `type_source` `annotation`. Связь, найденная только через типы из кода, SHALL иметь `resolution` `resolved` и `type_source` `code`. Если тип из кода противоречит аннотации, SHALL использоваться тип из кода.

#### Scenario: Локальная аннотация переменной

- **WHEN** в методе стоит `/** @var UserBonus $userBonus */`, затем `$userBonus = $freespin->userBonus;` и `$userBonus->updateWagerByTime();`, а тип `$freespin->userBonus` из кода неизвестен
- **THEN** ребро `calls` ведёт к `UserBonus::updateWagerByTime`
- **AND** у ребра `resolution` равно `annotated`, а `type_source` — `annotation`

#### Scenario: Свойство из докблока класса

- **WHEN** у класса `Freespin` есть `@property MaxWinUserBonus $maxWinUserBonus` и код вызывает `$freespin->maxWinUserBonus->getAllowedAmountToDep()`
- **THEN** существует вершина `property:Freespin::$maxWinUserBonus` с `magic: true` и `type_source: annotation`
- **AND** вызов распознан к `MaxWinUserBonus::getAllowedAmountToDep` с `resolution` `annotated`

#### Scenario: Метод из докблока класса

- **WHEN** у класса `yii\redis\Connection` есть `@method mixed exists($key, ...$keys)`, но нет объявленного метода `exists`, и код вызывает `$redis->exists('k')` на получателе этого класса
- **THEN** существует вершина `method:yii\redis\Connection::exists` с `magic: true` и `type_source: annotation`
- **AND** вызов распознан к ней с `resolution` `annotated`
- **AND** объявленный в PHP метод с тем же именем заменяет тег `@method`

#### Scenario: Код важнее аннотации

- **WHEN** переменной присвоено `new A()`, а аннотация говорит `@var B $x`
- **THEN** вызов `$x->run()` распознан к `A::run` с `resolution` `resolved`

#### Scenario: Тип из кода без аннотаций

- **WHEN** параметр объявлен как `A $a` и вызывается `$a->run()`
- **THEN** у ребра `resolution` равно `resolved`, а `type_source` — `code`

### Requirement: PLUGIN-09 Объединения, массивы и обобщения

Поиск метода и свойства SHALL рассматривать каждый класс из объединения типов, без `null`, `false` и скаляров. Тип элемента массива `X[]` SHALL передаваться переменной `foreach`. Возврат `static` и `$this` SHALL заменяться типом получателя, параметр `@template` — аргументом обобщения получателя.

#### Scenario: Цепочка ActiveQuery

- **WHEN** `ActiveRecord::find()` возвращает `ActiveQuery<static>`, `Query::where()` — `$this`, `ActiveQuery::one()` с `@template T` — `T|null`, а код пишет `$w = Wallet::find()->where([])->one(); $w->deposit(1);`
- **THEN** вызов `deposit` распознан к `Wallet::deposit` с `resolution` `annotated`

#### Scenario: Тип с null

- **WHEN** параметр объявлен `?A $a` и вызывается `$a->run()`
- **THEN** вызов распознан к `A::run` с `resolution` `resolved`

#### Scenario: Перебор массива объектов

- **WHEN** параметр аннотирован `@param A[] $items` и в `foreach ($items as $item)` вызывается `$item->run()`
- **THEN** вызов распознан к `A::run` с `resolution` `annotated`
