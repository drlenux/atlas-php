# Proposal: project-setup-file

## Why

All analysis and search settings are environment variables passed through `docker-compose.yml`
(and usually `.env`): about twenty `ATLAS_*` keys per consumer project. The owner wants them in the
project itself, in `./.atlas/setup.yaml`, with separate control of what atlas indexes, what text
search may scan, and what may never be read. Only secrets and resources stay in the environment.
This supersedes the RUNTIME-05 decision that every setting is an environment variable.

## What changes

- At startup atlas reads `<ATLAS_PROJECT_ROOT>/.atlas/setup.yaml` (optional, `version: 1`).
- Environment keeps: `ATLAS_DATABASE_URL`, `ATLAS_AGENT_TOKEN`, `ATLAS_REVIEW_TOKEN`,
  `ATLAS_CPU_LIMIT`, `ATLAS_MEMORY_LIMIT_MB`, `ATLAS_INDEX_WORKERS`, and the deployment wiring
  `ATLAS_PROJECT_ROOT`, `ATLAS_BIND`, `ATLAS_ALLOWED_ORIGINS` (derived from the published port).
- Every other setting lives in the file. When the file exists, a moved `ATLAS_*` key in the
  environment is a startup error naming the key and its YAML path. Without the file the old
  environment keys still work (deprecated).
- Scopes: `exclude` (never indexed, searched or read), `index.exclude` (graph only),
  `grep.exclude` (text search only). `.env` files are never indexed, searched or read.
- `atlas_read` refuses paths outside the readable scope.
- Compose files, `.env.example`, docs and an `examples/setup.yaml` follow; slotor is migrated.
