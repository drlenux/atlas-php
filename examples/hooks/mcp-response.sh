#!/usr/bin/env bash
# Example mcp.response_script: copy to <project>/.atlas/hooks/mcp-response.sh
# stdin = tool text, stdout = replacement. Env: ATLAS_MCP_TOOL, ATLAS_PROJECT_ROOT.
set -euo pipefail
# Identity filter (edit to drop lines, rewrite paths, etc.):
cat
