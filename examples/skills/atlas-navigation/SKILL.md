---
name: atlas-navigation
description: >-
  PHP/Yii2 code graph via Code Atlas MCP (atlas_search, atlas_outline, atlas_context,
  atlas_grep, atlas_read, atlas_batch). Use for every PHP callers/callees/fields/SQL/views
  question; load Atlas tools before shell grep. Includes hard checklists so agents do not
  skip called_by on factories or neighboring guards in action bodies.
---

# Code Atlas — куда смотреть и что проверять

Atlas индексирует PHP (Yii2): декларации + сайты
(`call_site`, `read_site`, `write_site`, `return_site`, `branch`, `sql_site`) с
`conditions`. MCP: `http://localhost:${ATLAS_HOST_PORT:-8080}/mcp`,
`Authorization: Bearer ${ATLAS_AGENT_TOKEN}`. Детали Yii/SQL: [reference.md](reference.md).

## 0. Перед любым PHP-поиском

1. **Сначала загрузи инструменты Atlas** (`ToolSearch` / enable MCP `atlas_*`).
   Bash `grep`/`rg`/`sed` по `*.php` без Atlas — ошибка процесса, не «быстрее».
   Если в проекте включён хук `php-search-hint.sh`, его подсказка — то же правило:
   вернись на `atlas_search` / `atlas_outline` / `atlas_grep`.
2. Если `# index state=` ≠ `completed` или файл в `pending=` — для этого файла
   временно Read/Grep, потом вернись в Atlas.
3. Независимые lookup'и пакуй в `atlas_batch` (до 10). У каждого вызова нужна
   обёртка `arguments:{…}` — без неё batch падает.

## 1. Жёсткие правила (из провалов агентов)

### 1.1 Фабрики и создатели — всегда вызывающие

Имена вроде `create*`, `activate*`, `issue*`, `prepare*`, `add*`, `claim*`,
`processing*`, плюс любой метод, который **создаёт** бонус/ФС/платёж/лимит:

1. Найти декларацию: `atlas_search(query:"Name", kinds:["method"], mode:"symbol")`
   или `Class::method` в `mode:"symbol"`.
2. **Сразу** `atlas_outline(method:…, sections:["called_by"])`.
3. **Читай строку `called_by` целиком**: в ней выражение вызова с аргументами
   (`$svc->createFreeSpin($user, $bonus)`) и `guards:[if(cond) return, …]` — до 3
   ближайших early-return перед вызовом. Разные аргументы/гарды = разные сценарии.
4. **`atlas_read` каждый caller из app-area** (`partner`, `api`, `admin`, `console`,
   `grcallback`, `common`), если гардов мало или вызов в длинном `action*`:
   `guards` — это только ближайшие 3 проверки, не всё тело.
5. Не останавливайся на первом пути (например только `partner`). Ошибка в
   `api/...` при том же `createFreeSpin` — типичный промах «только Atlas».

Пустой `called_by` ≠ unused: подтверди `atlas_grep` / `withGrep:true`
(`include_tests:true`, если нужны тесты).

### 1.2 Копируешь экшен / паттерн — читай тело целиком

Граф ведёт к **вызову** (`createFreeSpin`, мьютекс), но **не** к соседним проверкам
в том же `action*`. Дважды подряд агент с Atlas пропускал `isLockTakeBonuses`,
читая только куски по рёбрам.

Чеклист при новом контроллере / копировании activate-флоу:

- [ ] Открой **целиком** эталонный `action*` (`atlas_read` с запасом строк или файл)
- [ ] Выпиши все early-return / гарды до бизнес-вызова
- [ ] Типичное для бонусов/ФС: `isLockTakeBonuses`, верификация, самоограничение,
      лудоман, валюта/кошелёк, `canCreateMoreSimilar`, мьютекс + повторная проверка
      «сегодня» под lock, статусы ERROR / CANCEL_AUTO
- [ ] Сверь `guards:[…]` у callers в `called_by` с гардами эталона: расхождение —
      сигнал, что копия потеряла проверку
- [ ] Сверь, что твой код не выкинул ни один гард «потому что его не было в outline»

### 1.3 Расследование бага на проде

Нельзя закрыть задачу одним путём в графе:

1. Симптом → кандидатный метод/модель.
2. `called_by` + все области (см. 1.1).
3. Альтернативные входы: `api` vs `partner` vs `console` vs очередь/`execute`.
4. Состояние до/после вызова: `write_site` полей, Redis/кэш, транзакции.
5. Git (`blame`/`log`) — Atlas его не заменяет; для «кто внёс» нужен shell git.

## 2. Playbooks

### A. «Кто вызывает / мёртвый код»

```
search(symbol) → outline called_by (аргументы + guards в строке) → read сайты
если called_by пуст → atlas_grep word + include_tests
```

### B. «Цепочка до входа»

```
outline called_by (1 уровень) → context direction:"in" depth:3
не использовать context direction:"out" для callees
```

### C. «Где пишется поле»

```
search "Owner::$field" или FQCN, kinds:["write_site"], mode:"symbol"
для чтения — kinds:["read_site"]
смотри conditions на ветках
```

### D. «Спека / REQ-ID»

```
atlas_grep REQ-ID (MA-011, AU-009, BL-015…)
→ код + тесты (include_tests)
→ outline called_by на ключевые методы
→ сверка file:line со спекой (дрейф строк нормален — проверь символ)
```

### E. «View / partial»

```
search kinds:["view"] → outline(view:…, owner + called_by)
members view = params
controller members включают view:…
top-level в шаблоне = function:file:{path}; @var типизирует $this/$model
```

### F. «SQL / таблица»

```
outline(table:public.x) / column / property  (предпочтительнее grep)
maps_table / maps_column; sql_reads|sql_writes
createCommand/findBySql литералы (+ concat только из string literals)
```

## 3. Какой инструмент

| Вопрос | Вызов |
|---|---|
| Прямые callers / callees | `outline … called_by` / `calls` |
| Дерево callers | `context id direction:"in" depth:3` |
| Impls интерфейса | `outline method:I::m called_by + overridden_by` |
| Очередь | `outline …::execute called_by` — ищи `queue` |
| Класс по короткому имени | `search Name kinds:["class"] mode:"symbol"` — **всегда** `kinds` |
| `Class::method` | `search "UserBonus::closeBonus" mode:"symbol"` |
| Текст/комменты/конфиг | `atlas_grep` или `withGrep:true` |
| JS/CSS/переводы/git | shell Grep / git — не Atlas |

ID собирай явно: `class:…`, `method:…::f`, `property:…::$x`, `table:public.t`,
`view:path/to.php`. Отпечатки `call_site:` / `sql_site:` **только копируй** из ответа.

## 4. На что смотреть в ответе Atlas

- `resolved` / `annotated` (проверь PHPDoc) / `candidate` / `unresolved` / `external`
- `[unresolved: name only]` и `possible.name_only=N` — **не** доказанные callers
- `undeclared` — член точно отсутствует у известного receiver → runtime fail
- `conditions`: снаружи внутрь; `!(…)` = else
- Шум: magic `@property` в `outline`/`context` AR-класса — не путай с реальными вызовами
- Курсоры: `next_cursor` / `grepCursor` до `complete`
- Тесты исключены по умолчанию (`excluded.tests=N`)

## 5. Когда Atlas молчит — не выдумывай

| Симптом | Что сделать |
|---|---|
| `Storage operation failed` | Не крути retry-луп: перезапуск Atlas / другая нарезка `atlas_read`; продолжай Read/Grep |
| Пустой search по короткому имени | Уточни FQCN / `substring` / другое `kinds` |
| `atlas_grep` 0 по vendor | `include_vendor` или Read файла; vendor часто declaration-only |
| Лимит `context` у grep | Сузь путь/`include` или режь запрос |
| Нужна история коммита | `git log`/`blame` — вне Atlas |

## 6. Shell Grep — только здесь

- JS, CSS, ассеты, переводы, docblocks как текст
- Динамический SQL / QB без литеральных ключей
- Подтверждение «unused» и rename-аудит
- Git history

Для PHP-графа после нахождения символа **сразу возвращайся в Atlas** (outline/context).

## 7. Мини-чеклист перед ответом пользователю

- [ ] Atlas tools были загружены до PHP-grep
- [ ] У create/activate/factory есть полный `called_by` по area
- [ ] Эталонный экшен прочитан целиком, гарды не потеряны
- [ ] Для бага: проверен не один вход (api/partner/console/queue)
- [ ] Пустой `called_by` подтверждён grep'ом
- [ ] Спека: REQ-ID найден, file:line сверены с символом, не со старым номером строки
