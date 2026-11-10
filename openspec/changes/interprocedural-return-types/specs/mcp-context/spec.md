# MCP context — qualified symbol для occurrence sites

## ADDED Requirements

### Requirement: MCP-12 Symbol Owner::member для мест записи и чтения

В режиме `symbol` запрос вида `Owner::member` или `Owner::$member` (с необязательным namespace у Owner) SHALL находить occurrence sites (`write_site`, `read_site`, `call_site`), у которых имя члена совпадает с `member`, а `receiver_class` или `receiver_root_class` совпадает с Owner или оканчивается на `\Owner` (без учёта регистра).

#### Scenario: Запись поля по квалифицированному имени

- **WHEN** в индексе есть `write_site` с `member_name` `status` и `receiver_class` `common\models\Withdraw`
- **THEN** поиск `symbol` с query `Withdraw::$status` и kinds `write_site` возвращает этот site
- **AND** поиск `symbol` с query `common\models\Withdraw::status` и kinds `write_site` тоже возвращает его
