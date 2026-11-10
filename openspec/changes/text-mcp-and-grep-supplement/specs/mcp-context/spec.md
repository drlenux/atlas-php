## ADDED Requirements

### Requirement: MCP results are grep-style text
Every successful MCP tool result SHALL be plain text whose first line reports index
freshness. Node and grep results SHALL start with `path:line:` and SHALL carry kind, name,
resolution, conditions, remaining summary metadata, and the id and revision needed for
follow-up calls. The reviewer web API SHALL remain JSON.

#### Scenario: Search hit as a text line
- **WHEN** `atlas_search` returns a resolved call site at `common/models/BonusCod.php:399`
- **THEN** the result contains a line starting with `common/models/BonusCod.php:399: call_site`
  that includes `[resolved]`, its conditions as `if(...)` and `id=`

#### Scenario: Context edges reference listed nodes
- **WHEN** `atlas_context` returns an edge between two listed nodes
- **THEN** the edge line references them as `[n] -<kind>-> [m]`

#### Scenario: Web stays JSON
- **WHEN** the reviewer requests `/api/search`
- **THEN** the response is JSON with a `nodes` array

### Requirement: withGrep appends only what atlas did not return
`atlas_search` with `withGrep` SHALL run a text search over the source tree with the same
match rule as the search mode and SHALL omit lines covered by any atlas node that matches
the same query and kinds. The supplement SHALL be returned with the first node page or when
a grep cursor is given.

#### Scenario: Declaration line is not repeated
- **WHEN** atlas returns method `checkLifetime` declared at line 674 and grep finds line 674
- **THEN** line 674 is not in the grep supplement

#### Scenario: Coverage is not limited to the current page
- **WHEN** a call site matching the query is on the second atlas page
- **THEN** its line is not in the grep supplement of the first page

#### Scenario: Text mode grep matches whole identifiers
- **WHEN** the agent searches `BonusCod` in mode `text` with `withGrep`
- **THEN** a line containing only `BonusCodLogSearch` is not in the supplement

#### Scenario: Later node pages do not repeat the supplement
- **WHEN** the agent passes a node cursor without a grep cursor
- **THEN** the result has no grep supplement

### Requirement: Literal word grep is edge-aware
`atlas_grep` with `word` and a literal query SHALL require identifier boundaries only at
query edges that are identifier characters.

#### Scenario: Punctuation query with word
- **WHEN** `atlas_grep` searches literal `->pay(` with `word` true
- **THEN** a line `$w->pay();` is returned
