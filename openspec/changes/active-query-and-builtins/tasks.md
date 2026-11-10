# Tasks: active-query-and-builtins

- [x] 1 Тесты (red) и реализация PLUGIN-14: член `find`, `framework_returns`, точность в цепочке
- [x] 2 Тесты (red) и реализация PLUGIN-15: таблица встроенных классов
- [x] 3 `cargo test`, fmt, clippy, OpenSpec validate, замер на `slotor777`

## Итог замера на `slotor777`

- `processingGameTransaction`: unresolved 19 → 3, candidate 18 → 8 (annotated 79, resolved 57, external 33).
- `one()` по проекту: annotated 928, unresolved 32, candidate 20, resolved 6.
- `getMessage()`: 394 вызова `external` (`Exception::getMessage`, `Throwable::getMessage`), 5 `unresolved`.
- Строка 293 `getWallet()` — `candidate`: `api\modules\v1\models\User` переопределяет метод, это верный ответ. Строка 294 `deposit()` — `unresolved`: у `User::getWallet()` в PHPDoc стоит `@return object`, а вывода типа возврата из тела другого метода нет (тела анализируются независимо). Закрыто в `interprocedural-return-types`.
