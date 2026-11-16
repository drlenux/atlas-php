#!/usr/bin/env bash
# Retired: Atlas 0.3+ runs as two Compose services (atlas + atlas-db).
# Kept so old docs/scripts fail loudly instead of starting a monolith.
echo "code-atlas: monolithic entrypoint removed; use services atlas + atlas-db" >&2
exit 1
