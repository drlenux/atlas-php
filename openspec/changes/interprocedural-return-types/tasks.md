# Tasks: interprocedural-return-types

- [x] 1 OpenSpec artifacts + `openspec validate`
- [x] 2 Red-тесты PLUGIN-16/17 и MCP-12
- [x] 3 Второй flow-pass + `inferred_return` в `return_type()`
- [x] 4 Yii2 relation getter `framework_return_type` + instance `framework_return`
- [x] 5 Symbol `Owner::$member` в memory и PostgreSQL
- [x] 6 `cargo test` / fmt / clippy; openspec validate

## Итог

- `findOrCreate` без `@return` и `getWallet` с `@return object` → callers видят `Wallet`, `deposit`/`withdraw` в `called_by`.
- Relation getter → `ActiveQuery<Related>` на instance-вызовах.
- `atlas_search(Withdraw::$status, kinds:[write_site], mode:symbol)` находит write sites по receiver.
- На slotor777 нужна полная переиндексация; скилл atlas-navigation обновить отдельно.
