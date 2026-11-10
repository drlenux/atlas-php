# MCP context — свежесть файлов

## ADDED Requirements

### Requirement: MCP-13 Свежесть отдельных файлов

Блок `index` в ответах SHALL содержать `pending_paths` — файлы, изменённые после последней публикации, в том числе во время идущей сборки. `changes_pending` SHALL быть `true`, если этот список не пуст.

#### Scenario: Правка во время сборки

- **WHEN** файл изменён после начала сборки
- **THEN** во время сборки `changes_pending` равно `true`, а `pending_paths` содержит этот файл
