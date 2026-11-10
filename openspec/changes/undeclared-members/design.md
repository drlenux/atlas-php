# Design: undeclared-members

## Union property types (PHP plugin)

`Kind::Property` typing used `class_parts(..).find_map(lookup_property)`: the first part that has
the property decided the type. Decision: collect the property type of every part that declares it,
join distinct types with `|`; annotated when any contributing part is annotated.

## Undeclared members

Facts already on unresolved call metadata: `receiver_class`, `dispatcher` (`__call`/`__callStatic`
found in the hierarchy), `unindexed_ancestor`, `builtin`.

PHP plugin (`resolve`, no target found): `undeclared` when
- every receiver part is an indexed class-like declaration,
- no unindexed ancestor and no built-in declaring class,
- no `__call`/`__callStatic` in any part's hierarchy,
- no descendant of any part declares the method (polymorphic calls stay unresolved).
Then the call metadata gets `"undeclared": true` and the diagnostic `undeclared_member`
("Method C::m() is not declared in the class, its ancestors, traits or descendants").

Yii2 plugin (`enrich`, unresolved `calls` edge whose `dispatcher` is `yii\base\Component::__call`
and no other magic): after the existing behavior lookup finds no target and no unknown behavior,
and no descendant declares the method, it sets `undeclared: true` on the edge and its site node and
replaces the location's `unresolved_call` diagnostic with `undeclared_member`.

Presentation needs no change: boolean metadata prints as the bare key `undeclared`.

## Name-only callers (Service Layer)

`caller_sites` (outline `called_by`, callers tree root level) adds `call_site` nodes found by
`search_nodes(kinds:[call_site], mode:symbol, text:<method name>)` that have no outgoing `calls`
edge and an unknown receiver (`receiver_class` empty or `unknown`). Flag: `unresolved: name only`.
Noise bound: only when `search_nodes(kinds:[method,function], mode:symbol)` returns ≤ 3
declarations of that name; otherwise `Outline.name_only_callers` / `CallerTree.name_only_callers`
holds the count and the presenter prints it (`possible.name_only=N`).

## Grep scope

- `atlas_grep` gets `include_tests`; MCP passes the test-path regex as an exclusion to
  `grep_page`, which counts `excluded.tests`.
- `project::excluded` internal directories: add `.idea`, `.vscode`.

## Validation

Native PHP tests (union property, undeclared, `__call`, descendant, unindexed ancestor), Yii2
tests (behavior method, missing method on a component), service tests (name-only callers, noise
bound), grep tests (tests excluded/counted, IDE dirs), live slotor check against the manual list.
