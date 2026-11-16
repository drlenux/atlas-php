# Подключение Code Atlas к агенту (Claude Code / Cursor)

Проблема: агент «без указаний» берёт `grep`/`rg` по `*.php` и не вызывает Atlas, даже когда
в `CLAUDE.md` написано «mandatory». Текст не работает — нужна конфигурация. Три слоя,
ни один не блокирует работу по умолчанию:

| Слой | Где | Что делает |
|---|---|---|
| MCP `instructions` + описания `atlas_search/outline/grep` | сервер Atlas (`src/http.rs`) | Клиент без `CLAUDE.md` получает правило «Atlas вместо shell grep по PHP» при `initialize` и в `tools/list` |
| Soft-хук `hooks/php-search-hint.sh` | проект, `.atlas/hooks/` | На Bash/Grep по PHP добавляет подсказку агенту; режим `block` включается явно |
| Skill `skills/atlas-navigation/` | проект, `.claude/skills/` | Чеклисты: `called_by` с аргументами и `guards`, чтение тела экшена |

## Одной командой (slotor)

```bash
examples/install-agent-hints.sh /path/to/slotor
```

Скрипт копирует хук и skill, создаёт `.claude/settings.json` и `.cursor/hooks.json`
**только если их нет** (иначе печатает, что слить вручную). Код slotor не трогает.
Дальше нужен только MCP-сервер: [mcp.json.example](mcp.json.example) → `.mcp.json`
проекта (токен `ATLAS_AGENT_TOKEN` из `.env`). Новые `instructions`/описания появятся
после пересборки и перезапуска образа Atlas.

## Вручную

1. `cp examples/hooks/php-search-hint.sh <slotor>/.atlas/hooks/ && chmod +x <slotor>/.atlas/hooks/php-search-hint.sh`
2. Claude Code: слить [claude-settings.hooks.json](claude-settings.hooks.json) в
   `<slotor>/.claude/settings.json` (ключ `hooks.PreToolUse`).
   `additionalContext` у PreToolUse нужен Claude Code ≥ 2.1.9; на старых версиях подсказка
   агенту не видна — используйте `ATLAS_HOOK_MODE=block`.
3. Cursor: [cursor-hooks.json](cursor-hooks.json) → `<slotor>/.cursor/hooks.json`
   (событие `beforeShellExecution`; формат ответа Cursor проверен только по документации).
4. `cp -R examples/skills/atlas-navigation <slotor>/.claude/skills/`

`.atlas/` уже исключён из индексации и grep, поэтому хук не попадёт в граф.

## Поведение хука

Срабатывает, когда:

- Bash/Shell: `grep|egrep|fgrep|rg|ag|ack|git grep` с `*.php`/`--type php`, либо рекурсивный
  поиск (`rg …`, `grep -r …`) без указания не-PHP файла (`.js`, `.json`, `.md` …); а также
  `sed|awk|cat|head|tail|less|find|xargs` с явным `*.php`.
- Grep-инструмент: `glob`/`type`/путь указывают на PHP, либо поиск без `glob`/`type`.

Не срабатывает на `git log/blame`, поиск по JS/CSS/переводам, `Read`.

| ENV (в команде хука) | Значение |
|---|---|
| `ATLAS_HOOK_MODE=hint` | по умолчанию: подсказка, команда выполняется |
| `ATLAS_HOOK_MODE=block` | отказ (Claude: exit 2; Cursor: `deny`); обход — `atlas-skip` в команде |
| `ATLAS_HOOK_MODE=off` | хук выключен |
| `ATLAS_MCP_PREFIX` | префикс MCP-инструментов в тексте подсказки, по умолчанию `mcp__atlas__` |

Включить блокировку: в `settings.json` заменить `command` на
`ATLAS_HOOK_MODE=block "$CLAUDE_PROJECT_DIR"/.atlas/hooks/php-search-hint.sh`.

## Проверка

```bash
printf '%s' '{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"rg createFreeSpin api/"}}' \
  | .atlas/hooks/php-search-hint.sh      # → JSON с additionalContext
printf '%s' '{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}' \
  | .atlas/hooks/php-search-hint.sh      # → пусто
```

Метрика успеха — число вызовов `atlas_*` в повторном бенчмарке «без указаний»
(`scripts/agent_benchmark.py`, [docs/benchmark-agent.md](../docs/benchmark-agent.md)).
