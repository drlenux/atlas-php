# Analyzer plugins — межфункциональный return и relation getter

## ADDED Requirements

### Requirement: PLUGIN-16 Вывод типа возврата из тела для вызывающих

Если у метода нет informative declared/`@return` типа, анализатор SHALL вывести тип возврата из его body-returns и SHALL использовать его при разрешении вызовов в других функциях того же анализа.

#### Scenario: Фабрика без PHPDoc

- **WHEN** `Wallet::findOrCreate` не объявляет тип возврата и не имеет `@return`, а тело возвращает `Wallet`, и код пишет `$w = Wallet::findOrCreate(1, 'CP'); $w->deposit(1);`
- **THEN** вызов `deposit` распознан к `Wallet::deposit`

#### Scenario: Геттер с бесполезным `@return object`

- **WHEN** `User::getWallet` имеет `@return object` (или `@return object Wallet`), тело возвращает результат `getWallets()->…->one()`, а `getWallets` — relation `hasMany(Wallet::class)`, и код пишет `$w = $user->getWallet('CP'); $w->deposit(1);`
- **THEN** вызов `deposit` распознан к `Wallet::deposit`

### Requirement: PLUGIN-17 Тип возврата relation-геттера Yii

Для метода `getX`, из которого фреймворк вывел связь `hasOne`/`hasMany` к классу `Related`, вызов `$model->getX()` SHALL давать точный тип `yii\db\ActiveQuery<Related>` (цель вызова остаётся настоящим методом). Цепочка `->one()` / `->all()` SHALL получать модель / массив моделей.

#### Scenario: hasMany затем one

- **WHEN** `User::getWallets` возвращает `$this->hasMany(Wallet::class, …)`, а код пишет `$w = $user->getWallets()->andWhere([])->one(); $w->deposit(1);`
- **THEN** `getWallets()` уточнён к `ActiveQuery<Wallet>`
- **AND** `deposit` распознан к `Wallet::deposit`

### Requirement: PLUGIN-18 Return замыкания через mixed/callable-обёртку

Если вызов метода имеет не informative declared/`@return` (например `mixed`) и среди аргументов есть замыкание, анализатор SHALL типизировать результат вызова типом возврата этого замыкания.

#### Scenario: useMaster с use ($id)

- **WHEN** `Connection::useMaster` имеет `@return mixed`, а код пишет `$withdraw = $db->useMaster(function () use ($id) { return Withdraw::findOne($id); }); $withdraw->status = 1;`
- **THEN** запись распознана как `writes` к `property:…Withdraw::$status` (не `property_access` с `unknown`)

### Requirement: PLUGIN-19 class-string и динамический new

`@return class-string<T>` SHALL сохранять T (не схлопываться в `string`). `new $var` при типе переменной `class-string<T>` SHALL типизировать экземпляр как T. Чтения `Other::CONST` в RHS class-const SHALL попадать в граф как `reads`.

#### Scenario: gaminatorFamilyClass затем initGame

- **WHEN** резолвер возвращает `class-string<Gaminator>`, а код пишет `$c = Resolver::family(); $a = new $c([]); $a->initGame($game);`
- **THEN** вызов связан с `Gaminator::initGame` (не `unresolved` с receiver `…\$c`)

#### Scenario: MAP ключи-константы

- **WHEN** `private const MAP = [Keys::AGGREGATORUA => …]`
- **THEN** существует `reads` с `constant:…::MAP` на `constant:…Keys::AGGREGATORUA`
