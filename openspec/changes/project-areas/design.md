# Design: project-areas

## Setting

```yaml
areas:
  partner:    { paths: [partner] }
  admin:      { paths: [admin] }
  api:        { paths: [api] }
  grcallback: { paths: [grcallback] }
  console:    { paths: [console] }
  common:     { paths: [common], shared: true }
```

Prefix: `[a-z][a-z0-9_]*`; paths: relative folders. A path belongs to the area with the longest
matching folder prefix. Environment-only deployments have no areas.

Domain: `Area { prefix, paths, shared }` and `area_of(areas, path)` in `domain.rs`, shared by
config, analyzer options, services and HTTP (one rule, DRY).

## 1. Label and filter

- `PluginRegistry::analyze` stamps `metadata.area` on every node whose span path is in an area
  (language-neutral, after framework enrichment).
- Presenter: node lines `path:line: [area] kind name …` (the key is not repeated as
  `area=`); `CallerNode.area` from the caller site's metadata; tree lines `[area] Caller …`.
- `area` argument: the HTTP layer turns area names into an anchored path regex
  (`^(?:partner|admin)/`), passed as `NodeSearch.include_paths`, as a grep path filter, and as a
  caller filter for outline `called_by` and the callers tree. Unknown area names are argument
  errors that list the configured areas.

## 2. Yii application context

`components()` keeps per-file definitions. For an area A that is not shared, the component types
come from configs in A, in shared areas and outside every area; code in shared areas or outside
areas keeps the union. Framework member nodes (`property:yii\base\Application::$x`) carry
`types_by_area` next to the union `type`; PHP flow reads the area's type for a property read in a
function of that area. The enrich-time component resolution uses the same per-area definitions.

## 3. Namespace-less names

- `parse_file` gives the root `Names` the file's area; a class/function declared without a
  namespace in area A gets qualified name `A:Name` (id `class:A:Name`; `:` cannot occur in PHP
  names, so no collision with namespaces).
- After merging all files, a remap pass builds per-area tables (lowercase global name →
  qualified: a global outside every area (library) keeps its name everywhere, because two globals of one name cannot coexist at runtime; otherwise own area, then shared areas) and rewrites stored global references (parent,
  interfaces, traits, trait aliases, property/parameter/return types) and sets the table on every
  class/function `Names`, so `Names::resolve` maps global results while bodies are analyzed.
- Global function calls use the same table for the global fallback.
- Re-keying: when areas are first enabled, reconciliation would keep the old ids of renamed
  namespace-less entities (persistent ids survive renames). An incoming id that differs from an
  old one only by the inserted `<area>:` takes over unless the old object is protected (trust > 1
  or an endpoint of a protected edge).
- Limitation: Yii config `'class' => Name::class` with namespace-less names is not remapped.

## Validation

Config tests (areas parsing/validation), registry stamping test, presenter label tests, HTTP filter
tests (search/grep/outline/context), native PHP tests (same name in two areas, resolution order,
parent remap), Yii2 test (component class per area), live slotor with six areas.
