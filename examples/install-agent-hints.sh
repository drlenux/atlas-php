#!/usr/bin/env bash
# Installs the Code Atlas agent hints into a consumer project (never edits its code):
#   .atlas/hooks/php-search-hint.sh   soft PreToolUse hook
#   .claude/skills/atlas-navigation/  navigation skill
#   .claude/settings.json             hook registration, only if the file does not exist
#   .cursor/hooks.json                Cursor hook registration, only if the file does not exist
# Usage: examples/install-agent-hints.sh /path/to/project
set -euo pipefail

project="${1:?usage: $0 /path/to/project}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[[ -d "$project" ]] || { echo "not a directory: $project" >&2; exit 1; }

install -d "$project/.atlas/hooks" "$project/.claude/skills" "$project/.cursor"
install -m 0755 "$here/hooks/php-search-hint.sh" "$project/.atlas/hooks/php-search-hint.sh"
rm -rf "$project/.claude/skills/atlas-navigation"
cp -R "$here/skills/atlas-navigation" "$project/.claude/skills/atlas-navigation"

register() { # <source example> <target file>
  if [[ -e "$2" ]]; then
    echo "kept existing $2 - merge $1 into it by hand"
  else
    cp "$1" "$2"
    echo "created $2"
  fi
}
register "$here/claude-settings.hooks.json" "$project/.claude/settings.json"
register "$here/cursor-hooks.json" "$project/.cursor/hooks.json"
echo "done: hook + skill installed in $project (hint mode, nothing is blocked)"
