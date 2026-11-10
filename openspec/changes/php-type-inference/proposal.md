# Proposal

## Why

Замер на `slotor777` (обратная связь владельца, 2026-11-09): из 27 исходящих вызовов `BackgroundGameProcess::processingGameTransaction` распознано 3. Причины в анализаторе:

- аннотации `@var`, `@param`, `@return` и `@property` не читаются вообще. `$userBonus->updateWagerByTime()` не распознан, хотя рядом стоит `/** @var UserBonus $userBonus */`;
- методы ищутся по строке типа целиком, поэтому `X|null`, `A|B` и `X[]` никогда не находят класс;
- нет обобщённых типов и `static`, поэтому цепочка `X::find()->where()->one()` теряет тип на первом шаге;
- vendor не индексируется: «Definition not indexed: yii\db\ActiveRecord», и у `ActiveRecord::find()` нет объявления.

Владелец уточнил: тип из аннотации — не точный ответ, потому что аннотация может ошибаться. Результат, полученный через аннотацию, должен быть отмечен отдельно от выведенного из кода.

## What Changes

- Разбор PHPDoc: `@var` (у свойства и внутри тела функции), `@param`, `@return`, `@property`/`@property-read`/`@property-write` у класса, `@template`.
- Тип с происхождением: каждый выведенный тип знает, зависел ли он от аннотации. Код важнее аннотации: аннотация используется, только если код не дал информативного типа или если она уточняет тот же класс обобщённым параметром.
- Рёбра, распознанные через аннотацию, получают `resolution: "annotated"` вместо `"resolved"` и `type_source: "annotation"`. Вершины-свойства из `@property` помечены `type_source: "annotation"` и `magic: true`.
- Поиск метода и свойства по типам с объединением (`X|null`, `A|B`), массивам (`X[]`) и обобщённым параметрам (`ActiveQuery<X>`). Подставляются `static`/`$this` и параметры `@template`.
- `foreach` по `X[]` даёт элементу тип `X`.
- Индекс объявлений сторонних библиотек: пути из `ATLAS_DECLARATION_PATHS` (по умолчанию `["vendor/yiisoft"]`) разбираются без анализа тел функций. Получаются классы, методы, свойства и аннотации, но не места вызова.

## Capabilities

### Modified Capabilities

- `analyzer-plugins`: вывод типов из аннотаций с отметкой происхождения, объединения и обобщения.
- `project-indexing`: индекс объявлений сторонних библиотек.

## Impact

- `src/plugins/php/{syntax,flow,mod}.rs`, новый `src/plugins/php/doc.rs`; `src/analyzer/mod.rs` (опции), `src/config.rs`, `src/project.rs`, `src/indexer.rs`, `.env.example`, `docker-compose.yml`, `README.md`.
- Новое значение `resolution: "annotated"`: клиентам, которые фильтруют по `resolved`, нужно решить, включать ли его.
- Граф растёт за счёт объявлений vendor/yiisoft (около 530 файлов, без мест вызова).

Не входит (следующие change): связи ActiveRecord через `getX()`/`hasOne` без `@property`, иерархия как запрос (find implementations), компактная структура класса, режим поиска по символу и тексту, свежесть индекса.
