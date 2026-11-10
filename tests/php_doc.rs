//! php-type-inference: PHPDoc parsing and type normalization (pure, no project).
use code_atlas::plugins::php::doc::{doc_type, Access, DocBlock};

fn resolve(name: &str) -> String {
    let name = name.trim_start_matches('\\');
    match name {
        "UserBonus" => "common\\models\\UserBonus".into(),
        "ActiveQuery" => "yii\\db\\ActiveQuery".into(),
        _ if name.contains('\\') => name.into(),
        _ => format!("app\\{name}"),
    }
}
fn ty(raw: &str) -> String {
    doc_type(raw, &resolve, &[])
}

#[test]
fn normalizes_unions_nullable_scalars_and_class_names() {
    assert_eq!(ty("UserBonus"), "common\\models\\UserBonus");
    assert_eq!(ty("\\yii\\db\\Connection"), "yii\\db\\Connection");
    assert_eq!(ty("?UserBonus"), "common\\models\\UserBonus|null");
    assert_eq!(ty("UserBonus|null|NULL"), "common\\models\\UserBonus|null");
    assert_eq!(ty("integer|Boolean|double|string"), "int|bool|float|string");
    assert_eq!(ty("$this"), "static");
    assert_eq!(ty("static|self"), "static|self");
    assert_eq!(ty("positive-int"), "int");
    assert_eq!(ty("array{id: int, name: string}"), "array");
    assert_eq!(ty(""), "unknown");
}

#[test]
fn normalizes_arrays_and_generics() {
    assert_eq!(ty("UserBonus[]"), "common\\models\\UserBonus[]");
    assert_eq!(ty("array<UserBonus>"), "common\\models\\UserBonus[]");
    assert_eq!(ty("array<int, UserBonus>"), "common\\models\\UserBonus[]");
    assert_eq!(
        ty("list<UserBonus>|null"),
        "common\\models\\UserBonus[]|null"
    );
    assert_eq!(ty("iterable<string, int>"), "int[]");
    assert_eq!(ty("ActiveQuery<static>"), "yii\\db\\ActiveQuery<static>");
    assert_eq!(
        ty("ActiveQuery<UserBonus>"),
        "yii\\db\\ActiveQuery<common\\models\\UserBonus>"
    );
    assert_eq!(
        ty("\\yii\\console\\Application|\\yii\\web\\Application<TUserIdentity>|null"),
        "yii\\console\\Application|yii\\web\\Application<app\\TUserIdentity>|null"
    );
}

#[test]
fn template_names_are_not_resolved_as_classes() {
    let templates = vec!["T".to_string()];
    assert_eq!(doc_type("T|null", &resolve, &templates), "T|null");
    assert_eq!(doc_type("T[]", &resolve, &templates), "T[]");
}

#[test]
fn parses_declaration_tags() {
    let doc = DocBlock::parse(
        "*\n * Wallet model.\n *\n * @template T of ActiveRecord|array = ActiveRecord\n * @property int $id\n * @property-read MaxWinUserBonus $maxWinUserBonus relation\n * @property-write string $secret\n * @param UserBonus|null $bonus the bonus\n * @param array<int, UserBonus> $items\n * @return T|null a single row\n * @var UserBonus\n ",
        &resolve,
    );
    assert_eq!(doc.templates, vec!["T"]);
    assert_eq!(doc.properties.len(), 3);
    assert_eq!(doc.properties[1].name, "maxWinUserBonus");
    assert_eq!(doc.properties[1].type_name, "app\\MaxWinUserBonus");
    assert_eq!(doc.properties[1].access, Access::Read);
    assert_eq!(doc.properties[2].access, Access::Write);
    assert_eq!(doc.properties[0].access, Access::ReadWrite);
    assert_eq!(doc.params["bonus"], "common\\models\\UserBonus|null");
    assert_eq!(doc.params["items"], "common\\models\\UserBonus[]");
    assert_eq!(doc.return_type.as_deref(), Some("T|null"), "template kept");
    assert_eq!(doc.vars.len(), 1);
    assert_eq!(doc.vars[0].name, None);
    assert_eq!(doc.vars[0].type_name, "common\\models\\UserBonus");
}

#[test]
fn parses_inline_var_in_both_orders_and_prefers_tool_specific_tags() {
    let doc = DocBlock::parse("* @var UserBonus $userBonus ", &resolve);
    assert_eq!(doc.vars[0].name.as_deref(), Some("userBonus"));
    let doc = DocBlock::parse("* @var $bonus UserBonus", &resolve);
    assert_eq!(doc.vars[0].name.as_deref(), Some("bonus"));
    assert_eq!(doc.vars[0].type_name, "common\\models\\UserBonus");
    let doc = DocBlock::parse(
        "*\n * @return array\n * @phpstan-return list<UserBonus>\n ",
        &resolve,
    );
    assert_eq!(
        doc.return_type.as_deref(),
        Some("common\\models\\UserBonus[]")
    );
    let doc = DocBlock::parse("* plain text, no tags", &resolve);
    assert!(doc.is_empty());
}

#[test]
fn code_types_win_and_annotations_only_fill_or_refine() {
    use code_atlas::plugins::php::doc::refine;
    assert_eq!(refine("unknown", Some("A")), ("A".into(), true));
    assert_eq!(refine("mixed", Some("A|null")), ("A|null".into(), true));
    assert_eq!(refine("array", Some("A[]")), ("A[]".into(), true));
    assert_eq!(
        refine("A", Some("B")),
        ("A".into(), false),
        "contradiction: code wins"
    );
    assert_eq!(
        refine("A|null", Some("A")),
        ("A|null".into(), false),
        "no new information"
    );
    assert_eq!(
        refine("yii\\db\\ActiveQuery", Some("yii\\db\\ActiveQuery<X>")),
        ("yii\\db\\ActiveQuery<X>".into(), true)
    );
    assert_eq!(refine("A", None), ("A".into(), false));
    assert_eq!(
        refine("unknown", Some("unknown")),
        ("unknown".into(), false)
    );
}

#[test]
fn parses_method_tags() {
    let doc = DocBlock::parse(
        "*\n * @method mixed exists($key, ...$keys) Determine if a key exists.\n * @method static UserBonus make()\n * @method send(string $to)\n ",
        &resolve,
    );
    let methods: Vec<_> = doc
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.return_type.as_str(), m.is_static))
        .collect();
    assert_eq!(
        methods,
        [
            ("exists", "mixed", false),
            ("make", "common\\models\\UserBonus", true),
            ("send", "unknown", false)
        ]
    );
}
