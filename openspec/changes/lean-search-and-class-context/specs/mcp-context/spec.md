## ADDED Requirements

### Requirement: Text search matches whole identifiers
`atlas_search` mode `text` SHALL match the query only where its identifier-character edges
are not adjacent to other identifier characters. Mode `substring` SHALL keep plain substring
matching. The reviewer web search SHALL default to `substring`.

#### Scenario: Longer identifiers are not returned
- **WHEN** the agent searches `getUserBonus` in mode `text`
- **THEN** `getUserBonus` declarations and `->getUserBonus()` sites are returned and
  `getUserBonusCod` and `$targetUserBonus` are not

#### Scenario: Substring mode keeps partial matches
- **WHEN** the agent searches `UserBonus` in mode `substring`
- **THEN** `getUserBonus` and `getUserBonusCod` are returned

#### Scenario: Variable scope is not a match
- **WHEN** the agent searches `getUserBonus` and a `$this` variable is scoped to that method
- **THEN** the variable is not returned

#### Scenario: Punctuation edges need no boundary
- **WHEN** the agent searches `->pay(` in mode `text`
- **THEN** a site with expression `$w->pay()` is returned

### Requirement: Summary nodes omit derivable qualified names
Summary projections SHALL omit `qualified_name` when it equals the node `id` without the
leading `<kind>:` prefix.

#### Scenario: Method summary
- **WHEN** a summary includes node `method:A::pay` with qualified name `A::pay`
- **THEN** the serialized node has no `qualified_name`

### Requirement: Agent context omits declaration members
MCP `atlas_context` SHALL NOT list declaration members through `contains` or
`available_via`. The root SHALL still return its own occurrence sites, branches and local
variables, and the response SHALL report the number of omitted root members.

#### Scenario: Class context omits members
- **WHEN** the agent requests `atlas_context` for a class with four members
- **THEN** no member node is returned and `omitted_members` is 4

#### Scenario: Method context keeps its own sites
- **WHEN** the agent requests `atlas_context` for a method that contains a read site
- **THEN** the read site is returned

### Requirement: Agent context follows class hierarchy only from the root
MCP `atlas_context` SHALL follow `inherits`, `implements` and `uses_trait` edges only from
the requested entity, so a shared parent or trait does not list its other users.

#### Scenario: Parent without siblings
- **WHEN** the agent requests `atlas_context` for a class whose parent has other children
- **THEN** the parent is returned and its other children are not
