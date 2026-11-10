## ADDED Requirements

### Requirement: Source text search via MCP
The system SHALL provide an MCP tool `atlas_grep` that searches live project source
files (including files with no graph nodes) for a non-empty query.

#### Scenario: Literal hit in config without graph nodes
- **WHEN** the agent calls `atlas_grep` with query `piastrixInvoiceRefund`
- **THEN** a hit in `common/config/params.php` is returned even if that file has no atlas nodes

#### Scenario: Empty completed search
- **WHEN** the query matches nothing and every intended file was scanned without error
- **THEN** `hits` is empty, `truncated` is false, `incomplete` is false

#### Scenario: Incomplete search is flagged
- **WHEN** at least one intended file is skipped for size or fails to read
- **THEN** the response lists it under `skipped` or `errors` and sets `incomplete` true

### Requirement: Grep feature parity essentials
`atlas_grep` SHALL support literal and regex queries, optional case-insensitive and
word-boundary matching, path prefix/glob filters, optional vendor inclusion, output
modes `content`, `files_with_matches`, and `count`, and cursor pagination.

#### Scenario: Match-centered snippet
- **WHEN** a matching line is longer than the snippet budget
- **THEN** the returned `text` includes the match itself, not only the line prefix

### Requirement: Source read via MCP
The system SHALL provide `atlas_read` to return a live line range by path or by
graph `entity_id` span, so agents do not need an external Read/sed after search.

#### Scenario: Read method body by entity id
- **WHEN** `atlas_read` is called with a method `entity_id`
- **THEN** the response contains the current file lines covering that node's span

### Requirement: withGrep does not hide text inside declarations
When `atlas_search` uses `withGrep`, suppressed lines SHALL be limited to occurrence
site nodes already present in the page (`call_site`, `write_site`, `read_site`,
`return_site`, `url_site`, `branch`), not the full span of a class or method.

#### Scenario: Class node does not suppress inner comment match
- **WHEN** the page contains a `class` node spanning many lines and a comment inside
  that range matches the query
- **THEN** `withGrep` still returns that comment hit
