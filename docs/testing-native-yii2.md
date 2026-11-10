
## yii2-framework-members

Change: `openspec/changes/yii2-framework-members`.

1. `tests/yii2_members.rs` (6 сценариев PLUGIN-10, анализ через реестр). 4 упали (нет членов фреймворка), 2 отрицательных прошли сразу как ограничители. После реализации контракта `MemberSource`, слияния в PHP-плагине и членов Yii2 проходят 6 из 6. Старые `tests/native_yii2.rs` (10) не изменились: они вызывают `analyze` без шага членов.
2. Замер на `slotor777` показал 0 компонентов. Причина — динамическая карта в `scripts/qa-stand` и локальные записи без `class`. Добавлены 2 теста (запись без `class`, диагностика `dynamic_component_map`); оба упали, затем прошли.
3. Команды `yii\redis\Connection` объявлены только тегами `@method`. Добавлены тесты в `php_doc.rs` и `native_php_types.rs`; они упали (нет поля `methods`), затем прошли.

Замер на `slotor777` (`ATLAS_DECLARATION_PATHS=["vendor/yiisoft","vendor/2amigos"]`, `ATLAS_EXCLUDE=["scripts/qa-stand"]`): 290 свойств связей, 35 компонентов. Вызовы в проекте: resolved 34 928, annotated 11 217, candidate 4 821, unresolved 7 681 (до change — 8 042, до `php-type-inference` — 10 111), external 11 964. В `processingGameTransaction` unresolved 19. Остаются `\Exception::getMessage()` и цепочка `$user`. Команды Redis получают `candidate`: тестовые конфиги задают `redis` другим классом. Чтобы этого не было, исключите тесты через `ATLAS_EXCLUDE`. Индексация занимает 30 с.

```sh
cargo test --locked    # 149 passed, 5 ignored
```

## atlas-followups

Change: `openspec/changes/atlas-followups`. Повод — проверка владельца: у узла `call_site` и ребра разный статус, нет связи очереди с обработчиком, нераспознанные вызовы, тяжёлый `direction: "out"`.

Все исправления сделаны по TDD, red наблюдался по ожидаемой причине:

- публикация: узел с неизменённым fingerprint сохранял `unresolved`. Теперь сравнивается вся metadata. Повторная индексация `slotor777` без изменений не переписала ни одного объекта: revision всех вершин и рёбер остался 1;
- диагностики файлов с ошибкой разбора копились: было 3 копии, а на `slotor777` — 330 записей `parse_error` при 110 уникальных. Теперь 110 из 110;
- статус узла и ребра расходился в 4902 местах (3 причины). Теперь 0;
- очереди: `dispatches` с `queue` и `job_class`, в том числе для `push` с несколькими драйверами-кандидатами. У `GameProcess::execute` 15 входящих связей, все 15 вызовов `push` из отзыва;
- замыкания: `$wallet->deposit()` в `MyBonusController.php:2573` теперь `resolved`;
- константы: вершины `constant`. `property_access` в `processingGameTransaction` — 17 вместо 38, остался только `$user->…`;
- outline метода: `calls`/`called_by`. Для `processingGameTransaction` 18 КБ на 180 вызовов, а `direction: "out"` — 258 КБ.

Ограничение: `$user = User::find()->…->one()` при `User extends Da\User\Model\User` остаётся неизвестным. `find()` из `vendor/2amigos` возвращает `UserQuery` без параметра модели, поэтому нужно правило Yii «`X::find()` привязывает модель запроса к `X`».

```sh
cargo test --locked    # 166 passed, 6 ignored
```

## active-query-and-builtins

Change: `openspec/changes/active-query-and-builtins`.

1. `tests/active_query.rs` (PLUGIN-14). 2 из 3 упали: у `one()` были кандидаты из других классов запросов, а `getWallet`/`deposit` после `User::find()` из библиотеки не распознавались. Тест со своим классом запроса модели прошёл сразу, он оставлен как ограничитель. После реализации проходят 3 из 3.
2. `builtin_php_methods_are_external_with_their_return_types` (PLUGIN-15) упал. После реализации упал ещё раз из-за ошибки в таблице: методы были объявлены у интерфейса `Throwable`, а не у `Exception`. Исправлено, тест проходит.
3. Проверка «только объявления» перенесена в `AnalysisOptions::declaration_only` (было два одинаковых правила в плагинах).

Замер на `slotor777` — в `openspec/changes/active-query-and-builtins/tasks.md`.

```sh
cargo test --locked    # 186 passed
```
