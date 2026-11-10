# Proposal

## Why

Проверка владельца на `slotor777`: около 20% вызовов денежных методов `Wallet` отсутствуют в `called_by`. Ручная выборка — `RollbackController`, `BonusCod`, `Tournament`, welcome-bonus, `User` — все получают объект через `Wallet::findOrCreate(...)` или `$user->getWallet(...)`, а следующий `deposit`/`withdraw` остаётся `unresolved`.

Разбор:

1. Flow каждого метода идёт параллельно на объявлениях с пустыми `returns`. `return_type()` читает body-returns вызываемого метода, но к моменту анализа вызывающего их ещё нет и после merge они не перечитываются.
2. `Wallet::findOrCreate` без типа возврата и без `@return`: тип есть только в теле (`findOne` / `new Wallet`).
3. `User::getWallet` с `@return object Wallet` (парсер берёт `object`) и телом `getWallets()->one()`: relation даёт magic-property `$wallets`, но не уточняет возврат самого `getWallets()`, а `framework_return` сейчас только для статических вызовов (`X::find()`).
4. `atlas_search(Withdraw::$status, kinds:[write_site], mode:symbol)` пуст: symbol сравнивает `member_name` с целым запросом, `receiver_class` не участвует.

## What Changes

- PHP: после первого прохода flow вычислить `inferred_return` из body-returns, когда declared/doc не informative; второй проход flow использует его в `return_type()`.
- Yii2: для relation-геттера `getX` → member `kind: method` с `framework_return_type: ActiveQuery<Related>`; PHP применяет уточнение и к instance-вызовам.
- Search `symbol`: запрос `Owner::member` / `Owner::$member` находит occurrence sites по `member_name` + `receiver_class` / `receiver_root_class`.

## Capabilities

### Modified Capabilities

- `analyzer-plugins`: межфункциональный inferred return; instance `framework_return`; relation getter query type.
- `mcp-context`: qualified symbol search для write/read/call sites.

## Impact

- `src/plugins/php/{flow,mod,syntax}.rs`, `src/plugins/yii2/mod.rs`, `src/repository.rs`, `src/postgres.rs`
- Скилл `atlas-navigation` в slotor — после переиндексации (вне этого репозитория).
