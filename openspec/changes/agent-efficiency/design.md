# Design: agent-efficiency

## Components

```
MCP call_tool ──► atlas_batch ──► call_tool (each call, same analytics)
              ──► AtlasService use cases ──► mcp_text presenter
glob.rs: glob → anchored regex (memory matching, grep paths, PostgreSQL `~`)
project.rs: source walk with an optional extension filter (index: plugin extensions; grep: all)
```

## 1. atlas_batch

Problem: each call re-reads the agent context; grep agents batch shell commands.
Decision: tool `atlas_batch {calls:[{tool, arguments}]}` (1–10 calls, no nesting). Calls run in
order through the existing `call_tool`, so validation, analytics and errors are unchanged.
Output: one `# index …` line (taken from the first section), then per call
`## [n] <tool>` followed by its text without its own index line, or `## [n] <tool> error: …`.
Pattern: Composite over the existing command dispatcher; no new use case logic.

## 2. Grep scope, decoding and globs

- `iter_source_files(config, Option<&extensions>)`: `None` lists every regular file under the
  watch dirs with the same exclusion rules (internal dirs, vendor rule, `ATLAS_EXCLUDE`).
  Indexing passes plugin extensions; `SourceWalk::list_paths` (grep) passes `None`.
- `grep_page` reads bytes; a NUL byte in the first 8 KiB counts the file in
  `excluded.binary`; text is decoded with `String::from_utf8_lossy`, lines split on `\n`
  with a trailing `\r` removed.
- `ATLAS_GREP_MAX_FILE_BYTES` (u64, default 16 MiB, 1 KiB–1 GiB): larger files stay in
  `skipped` and make the result `incomplete`.
- `glob.rs`: `*` and `**` match any characters including `/` (keeps `*.sh` matching at any
  depth, as before), `?` one character, `[abc]`/`[!abc]` classes, `{a,b}` alternatives; the
  result is an anchored regex string usable by Rust `regex` and PostgreSQL `~`.
  `path_glob` is a string or a list; entries starting with `!` exclude.

## 3. Outline completeness

- `Outline.none: Vec<String>` (skipped in JSON when empty): sections that were requested and
  are empty. Explicit `sections` → every requested empty section; default method outline →
  `calls`, `called_by`; default class outline → `members`. Presenter prints `called_by: none`.
- PHP plugin: a method also `overrides` the same-named method of each interface the class
  implements directly (`lookup_method` on the interface). Abstract-class implementations then
  link concrete subclasses through the existing parent rule.

## 4. Tests, callers tree, class usages, literals, edge dedupe

Settings: `ATLAS_TEST_PATHS` (JSON array of globs, default
`["tests/*","*/tests/*","*Test.php","*Cest.php"]`), passed to `HttpConfig.test_paths`.

### Callers tree (MCP `atlas_context direction:"in"`)

Use case `AtlasService::callers_tree(CallersRequest{root_id, levels, max_nodes, tests})`
returns `CallerTree{root, children, truncated, excluded_tests, members_without_usages}` of
`CallerNode{label, id, path, lines, flags, conditions, expression, children, note}`.

- Shared `caller_sites(method)` (also used by outline `called_by`, DRY): incoming
  `calls`/`dispatches`/`magic_get` edges → site → owner via incoming `contains`.
- Level = one call level. Sites of one owner merge (`L667 L802 L946`); the first site supplies
  expression and conditions; flags are the union of resolutions.
- Owners that are methods/functions expand up to `levels` (default `min(3, max depth)`);
  recursion is cut with note `recursion`; the node budget is `max_context_nodes`.
- Test-path sites are dropped and counted in `excluded_tests` unless `include_tests`.
- Class-like root: children are members (methods: callers; properties/constants: reads and
  writes) used from outside the class; members without such usages are counted.

Presenter:

```
# callers <root id> levels=3 complete excluded.tests=6
<label> <path>:<line>
└ <caller> L497 [resolved] if(cond): <expression>
  └ …
```

### Search

- `NodeSearch.exclude_paths: Option<String>` (anchored regex built from `ATLAS_TEST_PATHS`);
  memory filters with `Regex`, PostgreSQL with `data->'span'->>'path' !~ $7`. MCP sets it
  unless `include_tests`; the `withGrep` coverage search clears it so test lines stay covered.
- Mode `text` matches `name`, `expression` and `value_expression` after removing the contents
  of quoted string literals (memory: scanner; PostgreSQL: `regexp_replace` with the same
  pattern). `substring` and `symbol` are unchanged.

### Edge dedupe (graph presenter)

Edge metadata keys whose value equals the source node's metadata are not printed, nor
`expression` equal to the source node name. Lossless: the values are on the node line.

## 5. Guidance and benchmark

- slotor `.claude/skills/atlas-navigation/SKILL.md`: batch, callers tree, mixed strategy
  (grep/`withGrep` for entry points, outline/tree for callers), narrow `atlas_read`.
- `docs/benchmark-agent.md` + script: the same questions to an atlas-only and a grep-only
  headless agent on two specs; tokens, calls, time and per-tool bytes recorded.

## Validation

TDD per behaviour: glob unit tests; grep scope/binary/lossy tests; batch HTTP test; outline
none and interface override tests (native PHP fixture); callers tree service tests; search
test-path and literal tests on memory and PostgreSQL; presenter tests; live slotor
re-measurement.
