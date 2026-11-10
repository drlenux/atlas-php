# Design: lean-search-and-class-context

## Search modes

```
MCP atlas_search / web /api/search ─► NodeSearch{mode} ─► Repository::search_nodes (memory | postgres)
```

| Mode | Match | Default for |
|---|---|---|
| `text` | query found in name / qualified name (declarations) / expression / value_* / member_name, with identifier boundaries at the query edges | MCP `atlas_search` |
| `substring` | same fields, plain substring (previous `text`) | web `/api/search` |
| `symbol` | unchanged exact match | — |

Identifier characters: ASCII letters, digits, `_`. A boundary is required only at an edge
whose query character is an identifier character, so `->pay(` or `Owner::member` still
match inside expressions, and `BonusCod` matches `common\models\BonusCod`.

Both repositories implement the rule (DRY at the contract level, one rule per engine):
- Memory: `contains_identifier(haystack, needle)` scanning occurrences with byte checks.
- Postgres: a `q` CTE builds one case-insensitive regex from the escaped query, with
  `(^|[^a-z0-9_])` / `($|[^a-z0-9_])` added per edge; `substring` mode uses the escaped
  query without boundaries. All text predicates use `lower(field) ~ q.pat`.

Ranking and cursor are unchanged.

Variables are matched by name only: their qualified name is the enclosing scope
(`method:A::b:$this`), so matching it returned every `$this` of a method named like the query.

Performance: each text predicate is `strpos(...)>0 AND field ~ pat`; the regex only runs on
substring candidates (live slotor: 0.43 s per search with regex alone).

## Summary qualified name

`project_node` (Detail::Summary) already clears `qualified_name` equal to `id` or `name`.
Add: equal to `id` with the leading `<kind>:` removed (e.g. `method:A::b` / `A::b`).

## Context members

Problem: membership edges (`contains`, `available_via`) from a class root list all
declarations; the root-only rule from `lean-grep-and-context` still lists them.

Decision: `Members::RootOnly` becomes `Expansion::Focused` (`Members::All` → `Expansion::Full`):
- outgoing membership edges of non-root nodes: skipped (unchanged);
- outgoing membership edges of the root: followed only when the target is an occurrence
  site, branch or `variable`; otherwise skipped and counted in `GraphContext.omitted_members`
  (serialized only when non-zero).
- `inherits` / `implements` / `uses_trait` are followed only from the root. Live slotor
  showed a class root reaching `ActiveRecord` and listing 99 other models through it.
`Expansion::Full` (web) is unchanged.

Omitted members are by contract and do not set `truncated`.

## Validation

- `tests/search_modes.rs` (memory): whole-identifier text, substring mode, punctuation queries.
- `tests/core_postgres.rs`: same rules on PostgreSQL (`atlas-pg-test`).
- `tests/lean_responses.rs`: summary drops kind-prefixed duplicate qualified name.
- `tests/context_members.rs`: class root omits members with count, method root keeps its sites.
- `tests/http.rs`: web search keeps substring default.
- Re-measure on slotor.
