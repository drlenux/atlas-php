# Proposal: agent-efficiency

## Why

Owner benchmark on slotor (spec `aggregatorua`, 6 questions, image 0.1.31): the atlas-only agent
used 117k tokens and 36 tool calls, the grep-only agent 68k tokens and 11 calls; both answered
6/6, atlas gave the complete caller list. Live checks confirmed the causes:

| Observation | Cause |
|---|---|
| one question = one call; grep batches 3–5 commands per Bash call | no batch tool |
| `atlas_grep path_glob:"*.sh"` → `scanned=0` | grep only scans plugin extensions (php, js, mjs, html, vue) |
| glob `[!p]` not supported | `path_glob` supports only `*` and `?` |
| `atlas_context` on a class → 1 node, `omitted_members=4` | members omitted, no class usages returned |
| Q4 "who calls `FreespinProviderInterface::updateLimits`" answered with a guess | empty outline sections are not printed; interface methods have no `overridden_by` (no `overrides` edges to interfaces) |
| `atlas_context direction:in depth:3` on `FreespinQuotaPolicy::allows`: 11.6 KB, misses callers of `createFreeSpin` | depth counts graph hops (site, owner) not call levels; owner classes, `available_via`, `contains` and repeated edge metadata; 7 of 40 lines tests |
| `atlas_search createFreeSpin kinds:[call_site]`: 30 rows, 11 real calls | matches inside string literals (`Logger::writeToLoki('createFreeSpin: …')`) and tests |

## What changes

1. `atlas_batch`: up to 10 tool calls in one request, one text result with numbered sections.
2. Text search scope and filters:
   - `atlas_grep` and the `withGrep` supplement scan every text file of the watched tree
     (binary files counted as `excluded.binary`), decode non-UTF-8 lossily, and use their own
     size limit `ATLAS_GREP_MAX_FILE_BYTES`;
   - globs support `*`, `?`, `[abc]`, `[!abc]`, `{a,b}`; `path_glob` accepts a list, entries
     starting with `!` exclude.
3. Outline: explicitly requested sections (and `calls`/`called_by` of the default method
   outline) print `<section>: none` when empty; methods implementing an interface method get
   `overrides` edges to it, so interface methods list `overridden_by`.
4. Agent views exclude tests by default (`ATLAS_TEST_PATHS`), with `include_tests:true` to keep
   them:
   - MCP `atlas_context direction:"in"` returns a callers tree whose depth counts call levels;
     several sites of one caller merge into one line; test callers are counted;
   - a class root returns its usages grouped by member (callers outside the class);
   - `atlas_search` excludes test paths (header `tests=excluded`), and mode `text` ignores
     matches inside string literals;
   - graph edges (directions `both`/`out`) omit metadata already printed on their source node.
5. Agent guidance (slotor `atlas-navigation` skill) and a repeatable benchmark on two specs.

## Out of scope

- New analyzer facts (class references without a constructor, type hints, `::class`).
- Web interface changes; the web API keeps its JSON shape.
