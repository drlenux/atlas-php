#!/usr/bin/env bash
# Soft PreToolUse hook (Claude Code / Cursor): when the agent searches or reads PHP
# through the shell (Bash grep/rg/sed/cat ... *.php) or the Grep tool, remind it to use
# Code Atlas MCP tools instead. By default it only adds a hint and NEVER blocks.
#
# Install: copy to <project>/.atlas/hooks/php-search-hint.sh (chmod +x) and register it
# via examples/claude-settings.hooks.json. See examples/README.md.
#
# ENV (set in the hook command, e.g. `ATLAS_HOOK_MODE=block .atlas/hooks/php-search-hint.sh`):
#   ATLAS_HOOK_MODE   hint (default) | block | off
#   ATLAS_MCP_PREFIX  MCP tool prefix shown in the hint (default mcp__atlas__)
# A command containing `atlas-skip` is always allowed (escape hatch for block mode).
#
# Needs only bash; payload is matched as raw JSON text, so no jq/python is required.
set -uo pipefail

mode="${ATLAS_HOOK_MODE:-hint}"
prefix="${ATLAS_MCP_PREFIX:-mcp__atlas__}"
[[ "$mode" == "off" ]] && exit 0

payload="$(cat)"
[[ "$payload" == *atlas-skip* ]] && exit 0

re_tool='"tool_name"[[:space:]]*:[[:space:]]*"([A-Za-z_]+)"'
re_command='"command"[[:space:]]*:[[:space:]]*"(([^"\\]|\\.)*)"'
re_event_claude='"hook_event_name"[[:space:]]*:[[:space:]]*"PreToolUse"'
re_grep_family='(^|[^[:alnum:]_./-])(grep|egrep|fgrep|rg|ag|ack|git[[:space:]]+grep)([^[:alnum:]_-]|$)'
re_always_recursive='(^|[^[:alnum:]_./-])(rg|ag|ack|git[[:space:]]+grep)([^[:alnum:]_-]|$)'
re_grep_recursive='grep[^|;&]*[[:space:]](-[a-zA-Z]*[rR][a-zA-Z]*|--recursive)([[:space:]]|$)'
re_file_readers='(^|[^[:alnum:]_./-])(sed|awk|cat|head|tail|less|find|xargs)([^[:alnum:]_-]|$)'
re_php='\.php|--type[= ]php|-t[[:space:]]*php|"type"[[:space:]]*:[[:space:]]*"php"'
re_non_php='\.(js|jsx|ts|tsx|vue|css|scss|json|md|ya?ml|twig|po|sql|txt|log|lock|xml|html)([^[:alnum:]]|$)'

tool=""
[[ "$payload" =~ $re_tool ]] && tool="${BASH_REMATCH[1]}"

command=""
[[ "$payload" =~ $re_command ]] && command="${BASH_REMATCH[1]}"
# Cursor beforeShellExecution carries only {"command": ...} without a tool name.
[[ -z "$tool" && -n "$command" ]] && tool="Bash"

matched=0
case "$tool" in
  Bash|Shell)
    if [[ "$command" =~ $re_grep_family ]]; then
      if [[ "$command" =~ $re_php ]]; then
        matched=1
      elif [[ ! "$command" =~ $re_non_php ]] &&
        { [[ "$command" =~ $re_always_recursive ]] || [[ "$command" =~ $re_grep_recursive ]]; }; then
        matched=1 # recursive search over the tree without a non-PHP file hint
      fi
    elif [[ "$command" =~ $re_file_readers && "$command" =~ \.php ]]; then
      matched=1
    fi
    ;;
  Grep)
    if [[ "$payload" =~ $re_php ]]; then
      matched=1
    elif [[ ! "$payload" =~ \"(glob|type)\" && ! "$payload" =~ $re_non_php ]]; then
      matched=1 # unrestricted content search
    fi
    ;;
esac
[[ "$matched" -eq 1 ]] || exit 0

hint="Code Atlas: for PHP use MCP tools ${prefix}atlas_search / ${prefix}atlas_outline (sections:[called_by] = callers with args and guards) / ${prefix}atlas_grep / ${prefix}atlas_read instead of shell grep. If they are not loaded yet, load them first (ToolSearch query: atlas). Shell grep is fine for JS/CSS/translations/git. Skip this check by adding 'atlas-skip' to the command."

cursor=0
if [[ "$payload" == *'"cursor_version"'* ]] || { [[ -n "$tool" || -n "$command" ]] && [[ ! "$payload" =~ $re_event_claude ]]; }; then
  cursor=1
fi

if [[ "$mode" == "block" ]]; then
  if [[ "$cursor" -eq 1 ]]; then
    printf '{"permission":"deny","decision":"deny","user_message":"%s","agent_message":"%s","reason":"%s"}\n' "$hint" "$hint" "$hint"
    exit 0
  fi
  printf '%s\n' "$hint" >&2
  exit 2
fi

if [[ "$cursor" -eq 1 ]]; then
  printf '{"permission":"allow","decision":"allow","agent_message":"%s"}\n' "$hint"
else
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"%s"}}\n' "$hint"
fi
exit 0
