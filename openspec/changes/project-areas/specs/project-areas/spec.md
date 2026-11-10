## ADDED Requirements

### Requirement: Areas label and filter entities
The system SHALL let `.atlas/setup.yaml` map area prefixes to folders, SHALL label every indexed
entity under an area with its prefix, and SHALL limit search, grep, outline callers and callers
trees to the areas named by the `area` argument.

#### Scenario: Label in results
- **WHEN** `partner: {paths: [partner]}` is configured and a search returns a method in
  `partner/controllers/A.php`
- **THEN** its line shows `[partner]`

#### Scenario: Filter by area
- **WHEN** the agent searches with `area: "admin"`
- **THEN** only entities under `admin/` are returned

### Requirement: Yii components follow the application of the area
The system SHALL type `Yii::$app` components in code of a non-shared area from the configuration
files of that area, shared areas and files outside every area.

#### Scenario: Different mailer per application
- **WHEN** `partner/config/main.php` sets `mailer` to `PartnerMailer` and `admin/config/main.php`
  to `AdminMailer`
- **THEN** `Yii::$app->mailer->send()` in `partner/` resolves to `PartnerMailer::send`

### Requirement: Namespace-less names are separated by area
The system SHALL prefix namespace-less classes and functions declared in an area with the area
prefix and SHALL resolve global names to a same-named global outside every area when one exists, otherwise to
the referencing area first, then shared areas.

#### Scenario: Same class in two areas
- **WHEN** `api/Provider.php` and `console/Provider.php` both declare a global `Provider`
- **THEN** the graph has `class:api:Provider` and `class:console:Provider`, and `new Provider()` in
  `api/` resolves to `api:Provider`
