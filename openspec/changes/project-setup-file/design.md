# Design: project-setup-file

## File

`<ATLAS_PROJECT_ROOT>/.atlas/setup.yaml` (the `.atlas` directory is already never indexed):

```yaml
version: 1
exclude: [docker/data_ignore, runtime]   # never indexed, searched or read
index:
  watch: ["."]
  exclude: ["*-local.php"]               # graph only
  include_vendor: false
  declaration_paths: [vendor/yiisoft]
  languages: [php]
  frameworks: [yii2]
  php_version: "8.2"
  max_file_bytes: 2097152
  max_analysis_bytes: 67108864
  max_graph_elements: 2000000
  analysis_timeout_secs: 120
  debounce_ms: 5000
  poll_ms: 1000
grep:
  exclude: ["*/tests/_output/*"]         # text search only
  max_file_bytes: 16777216
tests:
  paths: ["tests/*", "*/tests/*", "*Test.php", "*Cest.php"]
mcp:
  max_context_nodes: 100
  max_context_depth: 3
  max_request_bytes: 1048576
```

Every key is optional; omitted keys keep today's defaults. Unknown keys are errors
(`deny_unknown_fields`), so typos are reported with their location. YAML numbers for
`php_version` are accepted (`8.2`) as well as strings.

## Components

```
Config::from_map(env) ─► setup::read(root) ─► SetupFile (serde_norway, typed)
                        └► SetupFile::settings() → the same ATLAS_* key map → existing validation
```

Pattern: Adapter. The file is translated into the existing key/value settings, so every default,
range and validation rule stays in one place (`Config::from_map`). Errors that name a moved key are
rewritten to the YAML path (`index.watch in .atlas/setup.yaml`). File-only settings (`exclude`,
`grep.exclude`) are applied after validation.

Dependency: `serde_norway` (maintained fork of `serde_yaml`, MIT/Apache-2.0).

## Scopes

| Config field | Legacy environment | With setup.yaml |
|---|---|---|
| `excludes` (index walk) | `ATLAS_EXCLUDE` | `exclude` + `index.exclude` |
| `grep_excludes` (text search walk) | `ATLAS_EXCLUDE` | `exclude` + `grep.exclude` |
| `read_excludes` (`atlas_read`) | — | `exclude` |

Built-in for every walk and read: tool/VCS/IDE directories (existing list) and `.env` files
(`.env`, `.env.*` at any depth). `atlas_read` checks the relative path against the internal list,
`.env` files and `read_excludes` in `AtlasService::read_source` (Service Layer), with error
"path is outside the readable scope".

## Startup rules

- File missing → environment as before (deprecated path).
- File present → environment keys that moved to the file are rejected with one error listing them
  and their YAML paths. Secrets/resources/deployment keys stay in the environment.
- Changes to the file need a restart (documented).

## Validation

Config tests (file values, defaults, moved-key error, unknown key error, scope merge, YAML number
version), read-scope service test, grep-scope test, compose contract test; slotor migration and a
live restart.
