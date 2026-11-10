# Analyzer plugins — замыкания, константы, очереди

## ADDED Requirements

### Requirement: PLUGIN-11 Захваченные переменные замыканий

Замыкание SHALL получать типы переменных `use (...)` (для стрелочной функции — всех переменных области) из места определения.

#### Scenario: Кошелёк в замыкании

- **WHEN** метод принимает `Wallet $wallet` и определяет `function (int $x) use ($wallet) { $wallet->deposit($x); }`
- **THEN** вызов `deposit` распознан к `Wallet::deposit` с `resolution` `resolved`

### Requirement: PLUGIN-12 Константы классов

Константы классов, интерфейсов и `case` перечислений SHALL быть вершинами `constant`. Обращение `X::NAME` SHALL давать `reads` к константе с учётом родителей и интерфейсов или `constant_access` без цели, если константа не найдена. Обращение `X::class` SHALL NOT создавать рёбер доступа.

#### Scenario: Статус бонуса

- **WHEN** код читает `UserBonus::STATUS_END`, а класс объявляет `const STATUS_END = 3`
- **THEN** ребро `reads` ведёт к `constant:…UserBonus::STATUS_END` со значением `3`
- **AND** рёбер `property_access` для этого выражения нет

### Requirement: PLUGIN-13 Постановка задачи в очередь Yii2

Вызов `push($job)` метода очереди `yii\queue\Queue` (единственной цели или всех кандидатов) SHALL связываться ребром `dispatches` с методом `execute` класса первого аргумента, с именем компонента очереди. Ребро `calls` SHALL хранить типы аргументов с происхождением.

#### Scenario: Игровая транзакция

- **WHEN** код вызывает `Yii::$app->queueGame->push(new GameProcess([...]))`
- **THEN** у `GameProcess::execute` есть входящее ребро `dispatches` с `queue` `queueGame`

#### Scenario: Неизвестная задача

- **WHEN** в `push` передана переменная без известного класса
- **THEN** ребро `dispatches` не создаётся
