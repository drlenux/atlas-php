# Repository Guidelines

## Project Structure & Module Organization

This workspace contains a Rust Code Atlas service, native Rust PHP and Yii2 plugins, a PostgreSQL repository, and a web review/analytics interface. Implementation decisions and validation limits are recorded with the active OpenSpec change.

- `code-atlas-specification.md`: source requirements, version 0.5; distinguish confirmed requirements, preferences, and open questions.
- `openspec/config.yaml`: project configuration using the `spec-driven` schema; add confirmed project context here.
- `openspec/specs/`: accepted capability specifications, organized as `<capability>/spec.md`.
- `openspec/changes/<change-name>/`: proposals, specification deltas, designs, and task lists for active changes.
- `openspec/changes/add-code-atlas/`: requirements, `design.md`, `tasks.md`, source coverage in `traceability.md`, and current test evidence in `verification.md`. `open-questions.md` preserves original questions and points to implementation decisions.
- `openspec/changes/archive/`: completed changes retained for reference.
- `.agents/skills/`: generated Codex workflow skills after successful tool initialization.
- `src/`: Rust domain, Service Layer, PostgreSQL, HTTP/MCP, ENV, watcher/indexer.
- `src/analyzer/`: language-neutral LanguagePlugin/FrameworkPlugin contracts and registry.
- `src/plugins/php/`: native PHP language analysis; `src/plugins/yii2/`: separate native framework enrichment. No analyzed-language runtime or project autoloader may be invoked.
- `migrations/`: PostgreSQL schema; `web/`: shipped HTML/CSS/JS interface.
- `tests/`: behavior, HTTP, Git, indexing and integration tests; `docs/testing-*.md`: TDD evidence.
- `Dockerfile`, `docker/`: complete image and lifecycle; `docker-compose.yml` consumes ATLAS_IMAGE without building.
- `docker-compose.build.yml`: publisher-only build/push; `examples/compose.atlas.yml`: standalone consumer fragment.
- `.env.example`, `docs/distribution.md`: image release and namespaced consumer ENV contract; `examples/setup.yaml`: project settings template.

During the initial implementation the sandbox blocked root `.git` writes. Separate Git worktrees were used in an allowed temporary directory, with source copied into this workspace. `WORKTREE_LOCATION.txt` identifies that working repository when present; `artifacts/code-atlas.git.bundle` preserves its Git history when exported.

## Setup, Build, and Development Commands

Run commands from the workspace root. OpenSpec 1.13.1 was available during initial setup.

- `openspec init --tools codex --profile core --no-animation`: initialize or refresh Codex integration; requires write access to `.agents/`.
- `openspec list`: list active changes.
- `openspec list --specs`: list accepted specifications.
- `openspec new change add-memory-search`: create a named change workspace.
- `openspec status --change add-memory-search`: inspect artifact completion.
- `openspec validate --all --strict --no-interactive`: validate existing specifications and changes.

- `cargo test --locked`: Rust behavior, native PHP/Yii and in-process integration tests; no PHP interpreter, Composer or Git provenance.
- `node --test web/model.test.mjs web/indexing.test.mjs web/explore.test.mjs`: dashboard, indexing polling and code-explorer model tests.
- `cargo test --locked --test native_runtime`: empty-PATH analysis and runtime packaging checks.
- `cargo run --locked -- plugins`: list independently registered native language/framework plugins without DB.
- `ATLAS_TEST_DATABASE_URL='<disposable database>' cargo test --locked --test core_postgres -- --ignored --test-threads=1`: explicit real PostgreSQL integration checks.
- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`: Rust formatting and lint.
- `python3 tests/compose_distribution.py`: daemon-free external-project Compose contract checks (requires Docker Compose CLI and Python only for development).
- `docker compose config --quiet`, `docker compose pull atlas`, `docker compose up -d --no-build atlas`: install a published image.
- `docker compose -f docker-compose.build.yml build atlas` and `docker compose -f docker-compose.build.yml push atlas`: publisher commands after setting ATLAS_IMAGE; no runtime credentials required.
- `cargo run --locked -- serve`: local server/watcher after installing libpq and configuring ENV; `index`, `migrate`, and `stdio` are also available.

## Implementation Approach and Priority

Apply the following approaches in the owner's stated order of priority:

1. **Design architecture first.** Before implementation, describe the relevant component boundaries, responsibilities, dependency direction, data flow, and contracts in the change's `design.md`. Resolve decisions affecting the behavior being implemented before writing its production code.
2. **Design patterns.** Choose patterns to solve concrete problems identified by the architecture. Record the problem and reason for each choice; do not introduce patterns only for their own sake.
3. **TDD.** Implement each behavior through red-green-refactor: first a behavior-focused test and an observed failure for the intended reason, then the minimum implementation, then refactoring with passing tests. Derive tests from OpenSpec scenarios.
4. **Service Layer.** Put application use cases and their orchestration in a service layer. MCP and web handlers delegate to that layer; persistence and other infrastructure concerns stay behind the selected contracts. Share use-case behavior across entry points.
5. **SOLID + DRY + KISS + YAGNI.** Keep responsibilities and dependencies clear, avoid duplicated business rules, use the simplest adequate design, and add abstractions for present requirements rather than hypothetical features.
6. **Clean Simple Code.** Use explicit names, short cohesive operations, readable control flow, and consistent error handling. Keep implementation easy to inspect and change.

This priority order guides design trade-offs; all six approaches apply together. It does not select a programming language, concrete patterns, libraries, or the plugin protocol. See the source requirements, section 15.

Analysis, search and MCP settings live in the consumer project's `.atlas/setup.yaml` (template `examples/setup.yaml`, change `project-setup-file`); they apply at startup without source changes or image rebuilds. Only secrets, CPU/memory limits and deployment wiring (`ATLAS_PROJECT_ROOT`, `ATLAS_BIND`, `ATLAS_ALLOWED_ORIGINS`) stay environment variables passed by `docker-compose.yml`. This owner decision supersedes the all-environment rule of `RUNTIME-05`; keep names, types, defaults and validation in `src/config.rs` / `src/setup.rs`.

## Coding Style & Naming Conventions

Use descriptive Markdown headings, concise requirements, and fenced code blocks for commands. Use two-space indentation in YAML and avoid tabs. Name capability and change directories in lowercase kebab-case, such as `memory-search` and `add-memory-search`.

Give every requirement at least one `#### Scenario:` with explicit conditions and observable outcomes. Keep accepted behavior in specifications and proposed changes in their change directory. Use rustfmt and Clippy for Rust. PHP fixtures are input data, never executable test runners.

## Testing Guidelines

Run strict OpenSpec validation before submitting specification changes. This checks specification structure, not implementation correctness. Rust uses Cargo integration tests for the core and both native plugins; the web model uses Node's test runner. No percentage coverage threshold is imposed. Keep unavailable PostgreSQL/Docker/browser checks distinct from passing in-memory and in-process tests.

Use the TDD sequence above for executable behavior changes. Record the relevant failing and passing checks; writing tests only after production code is not TDD. Documentation-only changes require specification validation rather than artificial application tests.

## Commit & Pull Request Guidelines

Use short, imperative commit subjects, for example `docs: describe memory search requirements`. Keep each commit focused on one change. Worktrees may share a repository but must use separate Cargo target directories to avoid same-package artifact collisions.

Pull requests should explain the purpose, reference the relevant OpenSpec change and any issue, and record validation results and known gaps. Include screenshots only for visible interface changes.
