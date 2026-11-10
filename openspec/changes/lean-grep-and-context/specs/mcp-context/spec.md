## ADDED Requirements

### Requirement: Grep reports exclusions as counts
`atlas_grep` and `atlas_search` `withGrep` SHALL report files excluded by scope rules
(such as vendor exclusion) as a count per reason, and SHALL list individual files in
`skipped` only when skipping them makes the scan incomplete.

#### Scenario: Vendor files excluded by default
- **WHEN** the watched tree contains 3 vendor files and `include_vendor` is false
- **THEN** the response has `excluded.vendor_excluded` equal to 3, no `skipped` row with
  reason `vendor_excluded`, and `incomplete` false

#### Scenario: Oversized file still listed
- **WHEN** a file exceeds the size limit
- **THEN** it is listed in `skipped` and `incomplete` is true

### Requirement: Agent context expands member listings only for the root
MCP `atlas_context` SHALL follow outgoing `contains` and `available_via` edges only from
the requested entity. Owners reached through incoming membership edges SHALL be returned
without listing their members. The reviewer web context SHALL keep full expansion.

#### Scenario: Method context omits sibling members
- **WHEN** the agent requests `atlas_context` for a method whose class has other methods
- **THEN** the class node is returned and the sibling methods are not

#### Scenario: Class context member listing is superseded
- **WHEN** the agent requests `atlas_context` for a class
- **THEN** member listing follows `lean-search-and-class-context` (members omitted, counted)

#### Scenario: Reviewer keeps full view
- **WHEN** the web interface requests context for the same method
- **THEN** sibling members of the owner class are returned as before
