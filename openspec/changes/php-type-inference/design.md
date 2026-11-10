# Design: php-type-inference

## Context

Сейчас `flow.rs` хранит тип переменной строкой (`"A|null"`). `type_of(expr)` возвращает строку, а `resolve(call)` передаёт её в `lookup_method(classes, class, …)`, который ищет класс по строке целиком. Лексер сохраняет комментарии в `Unit.comments` (байтовые позиции), но парсер объявлений их не читает. Discovery исключает весь `vendor`, если не задан `ATLAS_INCLUDE_VENDOR`.

## Goals / Non-Goals

**Goals**

- Распознавать вызовы и обращения к свойствам через типы из PHPDoc и vendor-объявлений.
- Каждый результат, зависящий от аннотации, отличим от результата, выведенного из кода.

**Non-Goals**

- Полная система типов PHPStan/Psalm: условные типы, `key-of`, intersection, shapes и callable-сигнатуры.
- Связи ActiveRecord без `@property` (`getX()` → `hasOne(Cls::class)`); это следующий change в Yii2-плагине.
- Анализ тел функций в vendor.

## Architecture

```
tokens(Comment) ─► doc.rs: DocBlock::parse(text) + doc_type(raw, Names)
syntax.rs Parser ─► docblock перед объявлением ─► Function{param_docs, return_doc}, Property{doc_type}, Class{templates, magic properties}
mod.rs declarations ─► property/method/class nodes (+type_source)
flow.rs: Typed{name, annotated} ─► Ty-разбор (объединение / массив / обобщение) ─► lookup по каждому классу ─► resolution resolved|annotated|candidate
indexer/project ─► declaration_paths ─► AnalysisOptions.declaration_paths ─► PHP: без flow для этих файлов
```

Зависимости не меняются: `doc.rs` — чистый модуль без доступа к проекту, `syntax.rs` использует его при разборе, а `flow.rs` получает уже нормализованные строки типов.

## Типы

Канонические строки типов сохраняются: их уже используют metadata, тесты и Yii2-плагин. Новые формы:

- объединение `A|B|null` (как раньше);
- массив `X[]`: `array<X>`, `array<K, X>`, `list<X>`, `iterable<X>`, `X[]` нормализуются в `X[]`;
- обобщение `Base<Arg>`: хранится только первый аргумент (YAGNI: `ActiveQuery<X>` и подобные). Аргументы с несколькими параметрами сводятся к последнему (для `array<K,V>` это V);
- `$this`, `static`, `self` из PHPDoc приводятся к `static`/`self` и подставляются при вызове: `static` и `$this` становятся типом получателя (для `X::find()` получатель — `X`), `self` — классом, где объявлен метод.

Вспомогательные функции (`flow.rs`): `class_parts(ty)` возвращает классы из объединения без скаляров, `null`, `false` и массивов, `erase(part)` отбрасывает `<…>`, `element(ty)` даёт тип элемента `X[]`, `type_argument(part)` — аргумент обобщения, `substitute(ty, templates, receiver)` подставляет параметры и `static`.

## Происхождение (provenance)

`Typed { name: String, annotated: bool }` — результат `typed(expr, env)`. `Binding` получает поле `annotated`. Флаг распространяется: тип, вычисленный из выражения, отмечен, если отмечена хотя бы одна его составляющая (получатель, свойство, тип возврата). `merge` объединяет флаги через «или».

Правило «код важнее аннотации» (`refine(native, doc)`):

- если тип из кода неинформативен (`unknown`, `mixed`, `object`, `array`, `iterable`), берётся аннотация с флагом;
- если аннотация после `erase` даёт тот же класс, что и код, но уточняет его (`ActiveQuery` → `ActiveQuery<X>`, `array` → `X[]`), берётся аннотация с флагом;
- иначе используется тип из кода без флага.

Результат в графе:

| Объект | Поле |
|---|---|
| ребро `calls`/`reads`/`writes`, цель найдена через отмеченный тип | `resolution: "annotated"`, `type_source: "annotation"` |
| то же через тип из кода | `resolution: "resolved"`, `type_source: "code"` |
| свойство из `@property*` | вершина `property` с `metadata.magic = true`, `access`, `type_source: "annotation"` |
| свойство или параметр с типом из `@var`/`@param` | `type_source: "annotation"` |
| метод с `@return` | `metadata.annotated_return_type` |

`"candidate"` сохраняет прежний смысл (несколько реализаций) и тоже получает `type_source`. Диагностика `unresolved_call` выдаётся, как раньше, только для `"unresolved"`.

## Аннотации внутри тела

`/** @var Type $name */` (и `@var $name Type`) внутри функции записываются в `Scope.annotated_vars` при проходе через комментарий: `block()` перед каждой инструкцией разбирает ещё не обработанные комментарии. Чтение переменной использует аннотацию, если тип из кода неинформативен. Это одинаково работает для присваивания, `foreach` и использования без присваивания. `@var Type` без имени относится к переменной, которой присваивается значение в следующей инструкции.

## Docblock объявления

Для объявления на токене `i` парсер отступает через модификаторы (`public`, `protected`, `private`, `static`, `abstract`, `final`, `readonly`, `var`) и атрибуты `#[…]`. Docblock — последний комментарий `/** … */`, который заканчивается до первого токена объявления, причём между ними нет других токенов.

## Vendor-объявления

- ENV `ATLAS_DECLARATION_PATHS` — JSON-массив путей относительно корня, по умолчанию `["vendor/yiisoft"]`. Каждый путь проверяется: нормализованный, относительный, без `..`. Несуществующий путь пропускается без ошибки.
- Discovery добавляет эти каталоги, даже если `vendor` исключён. `AnalysisOptions.declaration_paths` передаёт их плагинам.
- PHP-плагин для таких файлов строит классы, методы, свойства, аннотации и связи наследования, но не запускает flow: нет мест вызова, переменных и возвратов. Ошибки разбора vendor-файлов остаются диагностикой.
- Если указан `ATLAS_INCLUDE_VENDOR=true`, vendor-файлы анализируются полностью, а `ATLAS_DECLARATION_PATHS` ничего не меняет.

## Patterns

- Value Object `Typed` — тип вместе с происхождением, чтобы флаг нельзя было потерять при передаче строки.
- Pure parser `doc.rs` — тестируется отдельно, без проекта.
- YAGNI: один аргумент обобщения, без intersection и conditional types.

## Risks / Trade-offs

- Неверная аннотация даёт неверную цель. Смягчение — `resolution: "annotated"` и `type_source` позволяют агенту отделить такие ответы, а код важнее аннотации.
- vendor-объявления увеличивают граф. Смягчение — только объявления и только `vendor/yiisoft` по умолчанию.
- Поиск кандидатов для неточного получателя перебирал все классы, а `lookup_method` перебирал имена классов без учёта регистра. На `slotor777` анализ превысил лимит 120 с, потому что получателей с известным типом стало много. Решение — `ClassIndex`: имена классов в нижнем регистре и заранее посчитанные конкретные наследники. Анализ функций теперь занимает около 11 с.
- `available_via` к унаследованным методам библиотек давал +148 тыс. рёбер (около 150 методов ActiveRecord на каждую модель). Такие рёбра не создаются: `lookup_method` находит эти методы и без них.
- Если иерархия получателя доходит до класса вне индекса (`Da\User\Model\User`), вызов остаётся `unresolved` (PLUGIN-07), но metadata `unindexed_ancestor` и текст диагностики называют этот класс. Тогда можно добавить пакет в `ATLAS_DECLARATION_PATHS`.

## Migration

Нужна полная переиндексация (кнопка «Переиндексировать» или изменение файлов).
