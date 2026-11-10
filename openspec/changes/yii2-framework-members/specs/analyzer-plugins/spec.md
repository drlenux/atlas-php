# Analyzer plugins — члены фреймворка до анализа тел

## ADDED Requirements

### Requirement: PLUGIN-10 Члены фреймворка в выводе типов

Framework-плагин SHALL иметь возможность до анализа тел функций добавить классам свойства, тип которых он устанавливает по коду или конфигурации. Языковой плагин SHALL использовать их при выводе типов переменных и цепочек. Такие свойства SHALL иметь `type_source` `framework`, признак фреймворка и вид доказательства. Объявленное в языке свойство SHALL быть важнее члена фреймворка, а член фреймворка — важнее аннотации.

#### Scenario: Связь hasOne через переменную

- **WHEN** у ActiveRecord-класса `Freespin` есть `getMaxWin()` с `return $this->hasOne(MaxWin::class, ['freespin_id' => 'id']);` и код пишет `$m = $freespin->maxWin; $m->getAllowed();`
- **THEN** существует свойство `Freespin::$maxWin` с типом `MaxWin|null`, `type_source` `framework`, `framework_evidence` `relation` и ребром `implemented_by` к `getMaxWin`
- **AND** вызов `getAllowed` распознан к `MaxWin::getAllowed` с `resolution` `resolved`

#### Scenario: Связь hasMany в foreach

- **WHEN** `getBonuses()` возвращает `$this->hasMany(UserBonus::class, [...])->inverseOf('freespin')` и код перебирает `foreach ($freespin->bonuses as $b) { $b->update(); }`
- **THEN** вызов `update` распознан к `UserBonus::update`

#### Scenario: Условный геттер

- **WHEN** геттер содержит `if` или возвращает `hasOne($class, …)` с переменной
- **THEN** свойство связи не создаётся

#### Scenario: Компонент приложения через переменную

- **WHEN** конфиг содержит `'components' => ['redis' => ['class' => 'yii\redis\Connection']]` и код пишет `$redis = Yii::$app->redis; $redis->get('k');`
- **THEN** существует свойство `yii\base\Application::$redis` с `framework_evidence` `component`
- **AND** вызов `get` распознан к `yii\redis\Connection::get`

#### Scenario: Неустановленная конфигурация компонента

- **WHEN** хотя бы один конфиг задаёт `components` динамически
- **THEN** свойства компонентов не создаются, и прежняя диагностика `ambiguous_component` сохраняется
- **AND** диагностика `dynamic_component_map` указывает файл и строку динамической карты

#### Scenario: Локальный конфиг без class

- **WHEN** `main.php` задаёт `redis` с `class`, а `main-local.php` задаёт `redis` только с `hostname` и `port`
- **THEN** свойство `redis` имеет класс из `main.php`

#### Scenario: Компонент с разными классами

- **WHEN** два конфига задают компоненту `mailer` разные классы с методом `send`
- **THEN** вызов `$mailer->send()` после `$mailer = Yii::$app->mailer` имеет `resolution` `candidate` и обе цели в кандидатах
