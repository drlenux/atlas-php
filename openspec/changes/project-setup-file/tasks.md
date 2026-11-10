# Tasks: project-setup-file

- [x] 1.1 Proposal, design, spec delta
- [x] 2.1 Red/green: setup.yaml values, defaults, moved-key and unknown-key errors
- [x] 2.2 Red/green: scopes (exclude / index.exclude / grep.exclude), `.env` never walked,
      grep scope also filters indexed paths
- [x] 2.3 Red/green: atlas_read refuses unreadable paths
- [x] 3.1 Compose files, .env.example, examples/setup.yaml, README, docs/plugins.md, AGENTS.md,
      compose contract (moved keys must not be passed), native_runtime expectation
- [x] 3.2 Migrate slotor (.atlas/setup.yaml, docker-compose.yml, .env), restart, verify

## Evidence (2026-11-10)

Checks: `cargo test --locked` 330 passed; `python3 tests/compose_distribution.py` 7 passed;
clippy and fmt clean.

Live slotor: the container environment holds only database URL, tokens, CPU/memory/workers,
project root, bind and origins; analysis settings come from `slotor/.atlas/setup.yaml`; startup
and full reindex succeeded; `atlas_read .env` is refused; `atlas_grep` reports
`excluded.tests=308` and no `.idea` paths.

Known gap: `examples/probe-project/docker-compose.yml` had three plugin/vendor keys whose original
values were not recoverable; its new `.atlas/setup.yaml` assumes `include_vendor: true` with
php/yii2, inferred from the probe README.
