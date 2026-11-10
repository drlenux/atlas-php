# Proposal

## Why

Проверка владельца на `slotor777`:

1. Самый частый шаблон Yii — `X::find()->…->one()` — выводится неоднородно: `candidate` для `Game`/`Freespin`, `annotated` без продолжения цепочки для `User`. Из-за этого не распознаны `$user->getWallet()` и следующий `$wallet->deposit()` (`BackgroundGameProcess.php:293–294`).
2. Все `$ex->getMessage()` в `catch (Exception $ex)` — `unresolved`.

Разбор:

- (1) Тип `ActiveQuery<Game>` не точный, поэтому `one()` получает кандидатов — переопределения `one()` в классах запросов gii (`BonusCodQuery`, `CountryQuery`, `LogQuery`…). Yii же гарантирует, что `X::find()` возвращает класс запроса, объявленный (или унаследованный) в `find()` модели. Для `User`: `find()` из `vendor/2amigos` возвращает `UserQuery` без параметра модели, и `one()` (`@return T|null`) не знает модели.
- (2) `getMessage()` объявлен во встроенном классе PHP `\Exception`, которого нет в индексе.

## What Changes

- Контракт членов фреймворка: член `kind: "method"` с `framework_return_type` уточняет тип результата вызова этого метода на данном классе-получателе, не меняя цель вызова. Признак `exact_return` делает результат точным: без кандидатов из наследников.
- Yii2: для каждой ActiveRecord-модели `X` проекта — член `find` с типом `Q<X>`. `Q` берётся из объявленного или аннотированного типа возврата `find()` модели или её предков, по умолчанию `yii\db\ActiveQuery`.
- PHP: точность (`exact`) переносится через `static`/`$this` в цепочке (`where()`, `andWhere()`…).
- PHP: таблица встроенных классов PHP (исключения и ошибки, `DateTime*`, `Closure`, `ArrayObject`/`ArrayIterator`). Если иерархия получателя доходит до встроенного класса с таким методом, вызов получает `resolution: "external"`, `builtin: "Exception::getMessage"` и тип возврата.

## Capabilities

### Modified Capabilities

- `analyzer-plugins`: уточнение типа результата фреймворком, встроенные классы PHP.

## Impact

- `src/plugins/php/{flow,mod,syntax}.rs`, новый `src/plugins/php/builtins.rs`, `src/plugins/yii2/mod.rs`, `src/analyzer/mod.rs` (описание контракта).
