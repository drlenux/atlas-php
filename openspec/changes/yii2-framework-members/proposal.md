# Proposal

## Why

После `php-type-inference` на `slotor777` остаются нераспознанными цепочки, тип которых определяет фреймворк, а не PHP:

- связи ActiveRecord без `@property`: `getMaxWinUserBonus()` возвращает `$this->hasOne(MaxWinUserBonus::class, …)`, значит `$model->maxWinUserBonus` имеет тип `MaxWinUserBonus|null`;
- компоненты приложения: `$redis = Yii::$app->redis; $redis->get(…)`, где класс `redis` задан в `components` конфига.

Yii2-плагин сейчас дораспознаёт только отдельные рёбра после анализа PHP (`Yii::$app->x->m()` прямо в одном выражении). Если значение сначала попадает в переменную или в цепочку, тип теряется.

## What Changes

- Контракт плагинов: новый шаг «члены фреймворка». После разбора объявлений и до анализа тел framework-плагин возвращает дополнительные свойства классов (language-neutral вершины `property`). PHP-плагин учитывает их при выводе типов.
- Yii2: свойство связи по геттеру `getX()` с единственным `return $this->hasOne|hasMany(Cls::class|'Cls', …)` (тип `Cls|null` или `Cls[]`).
- Yii2: свойство компонента приложения на `yii\base\Application` по однозначному `components` конфига (класс или объединение классов; неустановленная конфигурация не даёт свойства).
- Происхождение: `type_source: "framework"`, `framework: "yii2"`, `framework_evidence: "relation" | "component"`. Связь с геттером — ребро `implemented_by`. Фреймворк, выведенный из кода и конфига, важнее `@property`, но объявленное в PHP свойство важнее фреймворка.

## Capabilities

### Modified Capabilities

- `analyzer-plugins`: шаг членов фреймворка до анализа тел; связи ActiveRecord и компоненты Yii2.

## Impact

- `src/analyzer/{mod,registry}.rs`, `src/plugins/php/{mod,flow}.rs`, `src/plugins/yii2/mod.rs`.
- Новые вершины `property` с `magic: true` и рёбра `implemented_by`. Чтения связей становятся `reads` к свойству вместо `magic_get` к геттеру; геттер достижим через `implemented_by`.
- Не входит: `queue->push(new Job)` → `Job::execute`, `render` → view, `params`, `Yii::t`, маршруты и события (следующий change Yii2-связей).
