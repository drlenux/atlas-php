## ADDED Requirements

### Requirement: Project setup file
The system SHALL read analysis, search and MCP settings from `.atlas/setup.yaml` in the project
root when the file exists, keeping only secrets, resource limits and deployment wiring in the
environment.

#### Scenario: Settings from the file
- **WHEN** the file sets `index.debounce_ms: 1200`
- **THEN** the effective debounce is 1200 ms without any environment variable

#### Scenario: Moved key in the environment
- **WHEN** the file exists and the environment sets `ATLAS_WATCH_DIRS`
- **THEN** startup fails with an error naming `ATLAS_WATCH_DIRS` and `index.watch`

#### Scenario: Unknown key
- **WHEN** the file contains `index.wach`
- **THEN** startup fails with an error naming `wach`

#### Scenario: No file
- **WHEN** the file does not exist
- **THEN** the environment settings are used as before

### Requirement: Separate scopes for indexing, text search and reading
The system SHALL apply `exclude` to indexing, text search and `atlas_read`, `index.exclude` to
indexing only and `grep.exclude` to text search only, and SHALL never index, search or read `.env`
files.

#### Scenario: Grep-only exclusion
- **WHEN** `grep.exclude` contains `*/tests/_output/*`
- **THEN** `atlas_grep` does not scan those files and indexing still may

#### Scenario: Secret file
- **WHEN** the agent calls `atlas_read` with path `.env`
- **THEN** the call fails with "path is outside the readable scope"
