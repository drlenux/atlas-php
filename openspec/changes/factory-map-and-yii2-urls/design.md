# Design: factory-map-and-yii2-urls

## A. MAP factory

```
const MAP = [Keys::X => Foo::class, …]
        │
        ├─► read_site + reads → constant:Keys::X
        └─► metadata.class_string_values = [FQCN\Foo, …]
                    │
        $class = self::MAP[$k]  →  class-string<Foo>|…
                    │
        new $class(...)  →  constructed_type union (уже есть)
```

`constant_value_reads` эмитит occurrence `read_site` (не только edge с from=constant), чтобы search по `read_site` совпадал с grep.

Index access: если base — class const с `class_string_values`, тип элемента = union `class-string<T>`.

## B. Yii2 URLs

```
urlManager.rules / routes.php / Controller::actionY
        │
        ▼
   nodes kind=route  ──handles──► method:…::actionY
        ▲
 PHP Url::to / createUrl / string paths in .js .html .vue
        │
   url_site / call_site references
```

Discovery: при активном framework `yii2` registry/discovery добавляет extensions `js`, `html`, `vue` (и `mjs`). Эти файлы не идут в PHP analyze; Yii2 enrich сканирует строки.

Нормализация path: trim, без query/hash, без ведущего `/` для сравнения с route id; pattern rules хранят и pattern, и target route.

## C. Yii::createObject

`Yii::createObject(Foo::class)` (и `createObject($c)` при `$c: class-string<Foo>`) в `flow::typed` даёт тип `Foo`, даже если сам `createObject` в индексе возвращает `object`/`mixed`/`null`. Иначе `$x = Yii::createObject(…); $x->deposit()` остаётся `unresolved` с `receiver_class=unknown`.

## Risks

- Большие `.js` — те же ATLAS_EXCLUDE / max_file_bytes.
- Динамическая конкатенация URL — вне scope (только литералы и Url::to array).
- `Container::get` / конфиг-массив в createObject — вне scope (только `::class` и class-string).
