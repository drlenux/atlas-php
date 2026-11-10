# Yii2 URLs

## ADDED Requirements

### Requirement: YII-URL-01 Route nodes from rules and convention

Yii2 plugin SHALL создавать узлы kind `route` из `urlManager` `rules` / `routes.php` и канонические routes для `*Controller::actionY`, связывая их с action-методами ребром `handles`.

#### Scenario: Convention action

- **WHEN** объявлен `GameController::actionView`
- **THEN** существует `route:…:game/view` (или эквивалентный id) с `handles` на `method:…GameController::actionView`

### Requirement: YII-URL-02 PHP Url helpers reference routes

`Url::to`, `Url::toRoute` и `urlManager->createUrl` со статическим route string SHALL давать occurrence с `references` на соответствующий `route` (и action, если есть).

#### Scenario: Url::to array route

- **WHEN** код вызывает `Url::to(['game/view', 'slug' => $s])`
- **THEN** граф содержит ссылку на route `game/view`

### Requirement: YII-URL-03 URL sites in JS and HTML

При включённом yii2 discovery SHALL включать `.js`, `.html`, `.vue`; строковые path, совпадающие с известными routes, SHALL становиться `url_site` с `references` на `route`.

#### Scenario: fetch path

- **WHEN** в `.js` есть `fetch('/game/view')` и route `game/view` объявлен
- **THEN** существует `url_site` с `references` на этот route
