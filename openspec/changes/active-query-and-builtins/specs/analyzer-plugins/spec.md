# Analyzer plugins — запросы ActiveRecord и встроенные классы PHP

## ADDED Requirements

### Requirement: PLUGIN-14 Класс запроса модели Yii

Вызов `X::find()` для ActiveRecord-модели `X` SHALL давать точный тип `Q<X>`, где `Q` — класс запроса из `find()` модели или её предков (по умолчанию `yii\db\ActiveQuery`). Цель вызова SHALL оставаться настоящим методом `find`. Методы, возвращающие `$this`/`static`, SHALL сохранять точность, поэтому вызовы на результате не получают кандидатов из других классов запросов.

#### Scenario: Модель с запросом по умолчанию

- **WHEN** в проекте есть классы запросов, переопределяющие `one()`, а код пишет `$g = Game::find()->where([])->one(); $g->play();`
- **THEN** `one()` распознан к `yii\db\ActiveQuery::one` без кандидатов
- **AND** `play()` распознан к `Game::play`

#### Scenario: Модель со своим классом запроса из библиотеки

- **WHEN** `User` наследует библиотечную модель, чей `find()` возвращает `UserQuery` без параметра модели, а код пишет `$u = User::find()->where([])->one(); $u->getWallet()->deposit(1);`
- **THEN** `getWallet()` и `deposit()` распознаны

### Requirement: PLUGIN-15 Встроенные классы PHP

Вызов метода встроенного класса PHP из таблицы (в том числе через наследника из индекса) SHALL получать `resolution` `external`, имя встроенного метода и его тип возврата и SHALL NOT давать диагностику `unresolved_call`.

#### Scenario: Сообщение исключения

- **WHEN** код пишет `catch (\yii\base\Exception $ex) { $m = $ex->getMessage(); }`, а `yii\base\Exception` наследует `\Exception`
- **THEN** вызов `getMessage` имеет `resolution` `external` и `builtin` `Exception::getMessage`
- **AND** `$m` имеет тип `string`
