# Design: yii2-framework-members

## Context

`PluginRegistry::analyze` вызывает `LanguagePlugin::analyze` (объявления, затем тела функций), а потом `FrameworkPlugin::enrich` над готовым графом. Enrich переписывает только нераспознанные рёбра, и тип, который знает фреймворк, не попадает в переменные и цепочки PHP-анализа. В `php-type-inference` появились магические свойства из `@property`, а `property()`/`typed()` в `flow.rs` умеют их использовать.

## Goals / Non-Goals

**Goals**

- Тип, который фреймворк устанавливает по коду или конфигу, доступен PHP-анализу тел функций и проходит через переменные и цепочки.
- Контракт остаётся language-neutral: framework-плагин отдаёт вершины графа, а не внутренние структуры PHP-плагина.

**Non-Goals**

- Обобщённые геттеры Yii (`getFoo()` → `->foo` с произвольным возвратом). Их по-прежнему обрабатывает `enrich` (`magic_get`/`magic_set`, PLUGIN-04).
- Модули Yii (`Module::$components`), `Yii::$app->get('x')`, DI-контейнер.

## Architecture

```
Registry ─► LanguagePlugin::analyze_with_members(files, options, &FrameworkMembers)
PHP: parse ─► declarations ─► relations ─► members.members(&declarations_snapshot) ─► merge into Class.properties + nodes ─► flow ─► finalize
FrameworkMembers ─► for each selected FrameworkPlugin of the language: members(files, declarations, options)
Registry ─► FrameworkPlugin::enrich (как раньше)
```

Контракт (`src/analyzer/mod.rs`):

```rust
pub trait MemberSource { fn members(&self, declarations: &GraphSnapshot) -> Result<GraphSnapshot, String>; }
LanguagePlugin::analyze_with_members(&self, files, options, members: &dyn MemberSource) -> Result<GraphSnapshot, String>
    // по умолчанию: analyze(files, options) — языки без шага членов
FrameworkPlugin::members(&self, files, declarations, options) -> Result<GraphSnapshot, String>
    // по умолчанию: пустой снимок
```

Член фреймворка — вершина `kind: "property"`, `qualified_name: "<Class>::$<name>"`, metadata: `type`, `type_source: "framework"`, `magic: true`, `access`, `framework`, `framework_evidence` и необязательный `implemented_by` (id метода). Рёбра в возвращённом снимке не используются (YAGNI): `contains` и `implemented_by` строит языковой плагин по metadata. Так id и fingerprint рёбер остаются в одном месте.

Слияние в PHP-плагине:

1. класс берётся по `qualified_name` (без учёта регистра); если его нет, член пропускается;
2. если у класса есть объявленное в PHP свойство с тем же именем, член пропускается (код важнее);
3. магическое свойство из `@property` заменяется: фреймворк по коду важнее аннотации;
4. добавляются вершина, `contains` класс → свойство и `implemented_by` свойство → геттер.

Происхождение в `flow.rs`: чтение свойства отмечено как аннотация, только если `type_source == "annotation"` (раньше признаком было `magic`). Ребро чтения через член фреймворка получает `type_source: "framework"` и `framework`/`framework_evidence`, а `resolution` — `resolved`, если получатель не отмечен аннотацией. Тип из фреймворка не считается аннотацией при выводе типов.

## Yii2: связи

Кандидаты — методы `get<Name>` классов, для которых `is_a` проходит до `yii\db\BaseActiveRecord` или `yii\db\ActiveRecord` по цепочке `parent` (работает и без индекса vendor: проверяется имя предка). Тело (по span, как у `behaviors()`) должно содержать ровно одну инструкцию `return` верхнего уровня и не содержать условных конструкций. Выражение возврата начинается с `$this->hasOne(` или `$this->hasMany(`. После этого допускается цепочка `->via…`, `->inverseOf…`, `->andOnCondition…`, `->orderBy…` и подобных. Первый аргумент — `Cls::class` или строка с именем класса (разрешается через namespace и imports метода). Тип: `hasOne` → `Cls|null`, `hasMany` → `Cls[]`. Имя свойства — `<Name>` с первой буквой в нижнем регистре.

## Yii2: компоненты

Используется существующий `components()`. Член создаётся для каждого компонента, если в объявлениях есть класс `yii\base\Application`, глобальная карта компонентов не динамическая (`!unknown`), конфигурация компонента не содержит неустановленных частей, а классов хотя бы один. Несколько классов дают объединение: в разных конфигах разные классы, вызов получит `candidate`. Span указывает на файл и строку первого ключа `components`, где определён компонент.

Запись компонента без ключа `class` (например, `main-local.php` с `hostname`) не делает компонент неустановленным: Yii сливает её с записью, где `class` задан. Компромисс: если такая запись относится к другому приложению и рассчитывает на класс по умолчанию из `coreComponents`, атлас этого не увидит. Динамическая карта (`...$x`, переменная) по-прежнему делает неустановленными все компоненты (PLUGIN-04). Диагностика `dynamic_component_map` указывает место, чтобы нерабочие конфиги можно было исключить через `ATLAS_EXCLUDE`.

## Patterns

- Template Method по умолчанию в трейтах: языки и фреймворки без шага членов не меняются.
- Mediator `FrameworkMembers` в реестре собирает члены нескольких фреймворков одного языка; PHP-плагин не знает о Yii2.

## ENV

Новых переменных нет. Набор фреймворков по-прежнему задаёт `ATLAS_FRAMEWORK_PLUGINS`.

## Risks / Trade-offs

- Тело геттера ищется по span строк метода, как в `behaviors()`: несколько методов на одной строке дают пропуск, а не ошибку.
- Чтения связей становятся `reads` к свойству, а не `magic_get` к геттеру. Геттер остаётся достижим через `implemented_by`.
