## ADDED Requirements

### Requirement: Batched tool calls
The system SHALL provide `atlas_batch` that runs 1 to 10 atlas tool calls in order and returns
one text result with a section per call; a failing call SHALL produce an error section without
aborting the others.

#### Scenario: Two calls in one request
- **WHEN** the agent sends `atlas_batch` with an `atlas_outline` and an `atlas_grep` call
- **THEN** the result has one index line and sections `## [1] atlas_outline` and `## [2] atlas_grep`

#### Scenario: One failing call
- **WHEN** the second call names a missing entity
- **THEN** section 2 reports the error and section 1 still has its result

### Requirement: Grep scans every text file
`atlas_grep` and the `withGrep` supplement SHALL scan every non-binary file of the watched tree
that is not excluded by configuration, independent of analyzer file extensions.

#### Scenario: Shell script
- **WHEN** `deploy/run.sh` contains `aggregatorua` and the agent greps `aggregatorua` with
  `path_glob` `*.sh`
- **THEN** the hit in `deploy/run.sh` is returned

#### Scenario: Binary file
- **WHEN** a file contains a NUL byte
- **THEN** it is counted in `excluded.binary` and not reported as a match

#### Scenario: Glob class and negation
- **WHEN** `path_glob` is `["*.php", "!*/tests/*"]`
- **THEN** PHP files under `tests` directories are not scanned

### Requirement: Outline states empty requested sections
`atlas_outline` SHALL print `<section>: none` for every explicitly requested section that is
empty, and for empty `calls` and `called_by` of the default method outline.

#### Scenario: Interface method without callers
- **WHEN** the agent requests `sections:["called_by","overridden_by"]` for an interface method
  that nothing calls and one class implements
- **THEN** the result contains `called_by: none` and lists the implementing method under
  `overridden_by:`

### Requirement: Agent views exclude tests by default
MCP search, callers tree and outline callers SHALL leave out nodes under `ATLAS_TEST_PATHS`
unless `include_tests` is true, and SHALL say that tests were excluded.

#### Scenario: Test caller counted
- **WHEN** a method is called from production code and from a unit test
- **THEN** the callers tree lists the production caller and reports `excluded.tests=1`

### Requirement: Callers tree counts call levels
MCP `atlas_context` with direction `in` SHALL return callers as a tree where each level is one
call level, merging several sites of the same caller into one line.

#### Scenario: Three-level chain
- **WHEN** `C::c` calls `B::b` twice and `B::b` calls `A::a`, and the agent requests the
  callers of `A::a` with depth 2
- **THEN** the tree shows `B::b` under `A::a` and `C::c` with both lines under `B::b`

#### Scenario: Class usages
- **WHEN** the agent requests direction `in` for a class whose method is called from another class
- **THEN** the tree lists that method with the external caller

### Requirement: Text search ignores string literals
`atlas_search` mode `text` SHALL NOT match the query inside quoted string literals of names
and expressions.

#### Scenario: Log message mentions the method
- **WHEN** a call site `Logger::info('createFreeSpin failed')` exists
- **THEN** a text search for `createFreeSpin` does not return it
