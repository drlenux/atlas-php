# Proposal: project-areas

## Why

Projects split code into applications by folder (slotor: `partner`, `admin`, `api`, `grcallback`,
`console`, shared `common`); the split differs per project and must not be hard-coded. The owner
wants a prefix per folder in `.atlas/setup.yaml` that:

1. labels every entity of the folder and filters agent tools by it;
2. gives Yii code the configuration of its own application (`Yii::$app->x` from that
   application's configs, not the union of all configs);
3. separates same-named classes of namespace-less code (`api:Provider` vs `console:Provider`).

## What changes

- `areas` in `.atlas/setup.yaml`: prefix → `paths` (folders) and optional `shared: true`
  (code used by every area, e.g. `common`).
- Indexing stamps `metadata.area` on every node under an area. Text results show `[area]`
  after the location; callers-tree lines start with `[area]`.
- `area` argument (one or a list) on `atlas_search`, `atlas_grep`, `atlas_outline` and
  `atlas_context`: results limited to those areas' paths.
- Yii2: application components are typed per area from the area's configs plus shared areas and
  configs outside any area; code in shared areas or outside areas keeps the union.
- PHP: namespace-less classes and functions declared in an area get the qualified name
  `<prefix>:<Name>`; a global outside every area (library) keeps its name everywhere (so IDE stubs such as
  `common/config/__autocomplete.php` do not shadow `Yii`); other global names referenced from
  an area resolve to its own area first, then shared areas.
