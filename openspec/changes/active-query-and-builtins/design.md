# Design: active-query-and-builtins

## Уточнение типа результата фреймворком

`MemberSource` уже возвращает вершины-свойства. Новый вид члена — `kind: "method"`, `qualified_name: "<Class>::<method>"`, metadata: `framework_return_type`, `exact_return`, `framework_evidence`, `implemented_by`. PHP-плагин не создаёт для него вершину и не меняет цель вызова. Он записывает уточнение в `Class.framework_returns[method]`. При вызове `Class::method()` (статический получатель — сам класс, без учёта предков) тип результата берётся из уточнения, точность — из `exact_return`. Происхождение: если в типе есть параметр из PHPDoc (`one()` → `T|null`), результат по-прежнему отмечается аннотацией.

Почему не обобщённое правило PHP: «параметр шаблона = статический получатель» неверно в общем случае (фабрики коллекций). Это знание Yii, и потому оно приходит от Yii2-плагина.

## Точность в цепочке

`Typed` получает `exact`. Источники: `new X`, точная переменная (как раньше), уточнение фреймворка с `exact_return`. Возврат `static`/`$this` на точном получателе тоже точен. В `resolve()` получатель точен, если таков `Typed`. Кандидаты из наследников по-прежнему перебираются для неточных получателей и интерфейсов.

## Yii2: класс запроса модели

Для каждого класса проекта (вне `declaration_paths`), который `is_a` `yii\db\BaseActiveRecord`/`ActiveRecord`: метод `find` берётся у самого класса или ближайшего предка (`Index::methods`). Его `declared_return_type`, иначе `annotated_return_type`: первая класс-часть после стирания параметров, исключая `static`/`self`, и только если она `is_a` `yii\db\ActiveQuery` или `yii\db\ActiveQueryInterface`. Иначе — `yii\db\ActiveQuery`. Член: `framework_return_type = "<Q><X>"`, `exact_return = true`, `implemented_by` = найденный `find`.

## Встроенные классы PHP

`builtins.rs`: статическая таблица `класс → (родитель, интерфейсы, методы → тип возврата)` для `Throwable`, `Exception`, `Error`, `ErrorException` и стандартных SPL-исключений, `TypeError`, `ValueError`, `ArithmeticError`, `DivisionByZeroError`, `JsonException`, `DateTimeInterface`, `DateTime`, `DateTimeImmutable`, `DateInterval`, `DateTimeZone`, `Closure`, `ArrayObject`, `ArrayIterator`, `Countable`, `IteratorAggregate`, `JsonSerializable`, `Stringable`. Вершины не создаются: встроенный класс вне индекса проекта. В `resolve()`, если цели нет, а иерархия получателя доходит до неиндексированного класса из таблицы, где есть метод, вызов получает `resolution: "external"`, `builtin: "<Class>::<method>"`, а тип результата — тип из таблицы (как код). Диагностика `unresolved_call` для него не выдаётся.

## Patterns

- Уточнение через существующий контракт членов (Mediator из `yii2-framework-members`) вместо нового канала.
- Table-driven builtins (KISS): небольшой список, без генерации стабов.

## Risks / Trade-offs

- Таблица встроенных классов неполна: неизвестный метод встроенного класса остаётся `unresolved`, как раньше.
- Если модель возвращает из `find()` подкласс, не указанный в типе возврата, точность неверна. Это нарушение контракта Yii и встречается редко.
