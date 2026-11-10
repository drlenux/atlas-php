# Design: interprocedural-return-types

## Architecture

```
declarations + members
        │
        ▼
 flow pass 1 ──► metadata.returns per function
        │
        ▼
 inferred_return = refine(declared, doc); if !informative → union(body returns)
        │
        ▼
 flow pass 2 (same parallel host; Function.inferred_return set)
        │
        ▼
 finalize / sync_occurrence_resolution
```

`return_type(target)`: `refine(declared, doc)`; если результат не informative — `inferred_return`, иначе как сейчас (в т.ч. live `metadata.returns` того же прохода для ещё не сведённых случаев).

`framework_return`: для static — класс из имени получателя (как сейчас); для instance — первый class-part `call.matched`. Запись в `Class.framework_returns` без изменения цели вызова.

Yii2 `relation_members`: кроме property `$x` добавляет method-member `getX` с `framework_return_type: yii\db\ActiveQuery<Related>`, `exact_return: true` (тот же `ActiveQuery`, что у `find` по умолчанию — YAGNI на query-класс модели для связей).

## Symbol search

`Owner::member` / `Owner::$member` / `ns\Owner::$member`:

- declaration: прежнее точное `qualified_name` / `name`;
- occurrence (`call_site`/`read_site`/`write_site`): `member_name` == member (без `$`) и `receiver_class` или `receiver_root_class` равны owner или оканчиваются на `\owner` (без регистра).

Memory и PostgreSQL — одно правило.

## Patterns

- Two-pass analysis: второй проход переиспользует тот же worker path; результат первого графа отбрасывается, остаются только inferred returns (KISS, без частичного invalidation).
- Уточнение relation getter через уже существующий method-member канал (`merge_return_refinement`).

## Callable return passthrough

`$x = $db->useMaster(function () { return Withdraw::findOne($id); })`: у `useMaster` declared/`@return` = `mixed` (не informative). После анализа аргумента-замыкания (он уже разобран в `walk_expr` до `typed` присваивания) тип вызова SHALL браться из return замыкания (`declared` / body `metadata.returns`). То же для любого вызова с Closure-аргументом, пока собственный return callee не informative. `@var` на `$x` по-прежнему перекрывает.

## class-string dynamic new

`@return class-string<Gaminator>` не схлопывать в `string` в `doc_type`; `refine(string, class-string<T>)` берёт annotation. `new $var` при типе `class-string<T>` → экземпляр `T` (`exact: false`, чтобы потомки попали в candidates). Иначе `$aggregatorClass` резолвился как буквальное имя в namespace файла.

## Class const initializer reads

В RHS `private const MAP = [Keys::X => …]` эмитить `reads` с `constant:Owner::MAP` на `constant:Keys::X` (токен-скан `Ident::Ident`, пропуск `::class`).

## Risks

- ×2 время php_flow. На slotor777 приемлемо ради полноты `called_by`.
- Неверный body-return (динамический return) даёт неверный inferred; смягчение — только когда declared/doc не informative.
- `receiver_class` у write_site может быть предком/интерфейсом; суффикс `\Owner` покрывает FQCN и короткое имя в конце.
- Callable passthrough ошибочен, если метод глотает callback и возвращает другое; применяется только при неinformative return callee.
