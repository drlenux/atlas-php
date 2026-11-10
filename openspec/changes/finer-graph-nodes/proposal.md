# Proposal

## Why

Ответ `atlas_context` слишком тяжёл для агента: нода метода несёт весь текст тела, а вызовы и ветки остаются только рёбрами/условиями. Нужна более мелкая гранулярность вершин и явный признак закомментированного кода, чтобы сравнение с Serena/grep и денежное ревью по условиям пути были practically usable.

Источник: обратная связь владельца по probe на `BackgroundGameProcess::processingGameTransaction` / `GameProcess`; уточнение модели — вариант C (occurrence + branch) и флаг `commented` на нодах. В ТЗ §14 отдельные вершины на вызовы ранее не были обязательны — это изменение делает их обязательными.

## What Changes

- **BREAKING**: нода `method`/`function` больше не хранит полный `source`/`body`; текст кода живёт у более мелких вершин (occurrence, branch, return site) или читается по span из файла.
- Добавляются вершины `branch` для `if`/`elseif`/`else` (и аналогов с явным условием) с полем условий.
- Добавляются вершины occurrence (`call_site` / `write_site` / `read_site` / `return_site`) внутри метода; рёбра `calls`/`reads`/`writes` исходят от occurrence, а не от method.
- Method связан с occurrence/branch через `contains`.
- У каждой вершины появляется булев флаг `commented` (`true` — сущность извлечена из комментария, `false` — из исполняемого кода).
- Закомментированный PHP-подобный код (line/block comments), распознанный как объявление или occurrence, индексируется отдельными нодами с `commented: true`, чтобы grep-подобное покрытие «закомментированных вызовов» не терялось.

## Capabilities

### New Capabilities

Отсутствуют.

### Modified Capabilities

- `code-graph`: мелкая гранулярность вершин, флаг `commented`, перенос тела метода с method-ноды.
- `analyzer-plugins`: PHP-анализатор эмитит branch/occurrence и разбирает код в комментариях.
- `mcp-context`: контекст вокруг мелких вершин остаётся связным; ответ по method по умолчанию не раздувается телом.

Примечание: принятых specs в `openspec/specs/` ещё нет (change `add-code-atlas` не архивирован). Delta пишутся относительно capability-путей из `add-code-atlas`.

## Impact

- PHP-плагин (`src/plugins/php/`), частично Yii2 enrichment.
- Контракт graph snapshot / MCP JSON: клиенты, ожидающие `metadata.body`/`source` на method, сломаются.
- Тесты `tests/native_php.rs` и связанные fixture-сценарии.
- Размер БД: больше нод/рёбер, зато каждая нода существенно легче.
