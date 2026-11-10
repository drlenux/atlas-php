# Proposal

## Why

Остатки после 0.1.15: `FreespinProviderFactory` выбирает класс из `private const MAP` и делает `new $class` — константы в RHS MAP не видны как `read_site`, адаптеры не попадают в `called_by`. Параллельно агентам нужен граф Yii2 URL (гайд Url / urlManager) и поиск ссылок в PHP, JS, HTML.

## What Changes

- PHP: `read_site` для `Other::CONST` в RHS class-const; metadata `class_string_values` на MAP; `$x = self::MAP[$k]` → union `class-string`; `new $x` → candidates.
- Yii2: узлы `route` из urlManager rules + конвенция controller/action; `Url::to` / `createUrl` → `references`; discovery `.js`/`.html`/`.vue` + `url_site`.

## Capabilities

### Modified Capabilities

- `analyzer-plugins`: MAP read_site и class-string values.

### New Capabilities

- `yii2-urls`: маршруты и вхождения в PHP/JS/HTML.

## Impact

- `src/plugins/php/{mod,flow}.rs`, `src/plugins/yii2/`, discovery/registry, skill atlas-navigation.
