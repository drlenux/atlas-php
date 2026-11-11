use code_atlas::{
    analyzer::{AnalysisOptions, LanguagePlugin, SourceFile},
    plugins::{pgsql::PgsqlLanguagePlugin, php::PhpLanguagePlugin},
};

fn analyze_sql(source: &str) -> code_atlas::domain::GraphSnapshot {
    PgsqlLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "schema.sql".into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

fn analyze_php(source: &str) -> code_atlas::domain::GraphSnapshot {
    PhpLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "Wallet.php".into(),
                source: source.into(),
            }],
            &AnalysisOptions::default(),
        )
        .unwrap()
}

fn ids<'a>(kind: &str, g: &'a code_atlas::domain::GraphSnapshot) -> Vec<&'a str> {
    g.nodes
        .iter()
        .filter(|n| n.kind == kind)
        .map(|n| n.id.as_str())
        .collect()
}

#[test]
fn create_table_columns_and_references_become_graph() {
    let g = analyze_sql(
        r#"
CREATE SCHEMA app;
CREATE TABLE app.users (
    id bigint PRIMARY KEY,
    email text NOT NULL,
    org_id int REFERENCES app.orgs (id)
);
CREATE TABLE app.orgs (id int PRIMARY KEY, name text);
"#,
    );
    assert!(ids("schema", &g).contains(&"schema:app"));
    assert!(ids("table", &g).contains(&"table:app.users"));
    assert!(ids("column", &g).contains(&"column:app.users.id"));
    assert!(g.edges.iter().any(|e| {
        e.kind == "references"
            && e.from == "column:app.users.org_id"
            && e.to.as_deref() == Some("table:app.orgs")
    }));
}

#[test]
fn php_create_command_update_links_wallet_table() {
    // Pattern from slotor AggregatoruaController / ChampionController.
    let g = analyze_php(
        r#"<?php
namespace App;
class WalletService {
    function debit($db, $amount, $id) {
        return $db->createCommand(
            'UPDATE wallet SET amount = amount - :amount WHERE id = :id RETURNING amount',
            [':amount' => $amount, ':id' => $id]
        )->queryScalar();
    }
}
"#,
    );
    assert!(
        ids("sql_site", &g)
            .iter()
            .any(|id| id.contains("method:App\\WalletService::debit")),
        "expected sql_site in debit, got {:?}",
        ids("sql_site", &g)
    );
    assert!(ids("table", &g).contains(&"table:public.wallet"));
    assert!(g
        .edges
        .iter()
        .any(|e| { e.kind == "sql_writes" && e.to.as_deref() == Some("table:public.wallet") }));
}

#[test]
fn php_find_by_sql_and_variable_select() {
    let g = analyze_php(
        r#"<?php
namespace App;
class Repo {
    function lock($id) {
        return UserBonus::findBySql('SELECT * FROM user_bonus WHERE id = :id FOR UPDATE', [':id' => $id])->one();
    }
    function scan($db) {
        $query = "SELECT DISTINCT session_data FROM game_session WHERE id > :id";
        return $db->createCommand($query)->queryColumn();
    }
}
"#,
    );
    assert!(ids("table", &g).contains(&"table:public.user_bonus"));
    assert!(ids("table", &g).contains(&"table:public.game_session"));
    assert!(g
        .edges
        .iter()
        .any(|e| { e.kind == "sql_reads" && e.to.as_deref() == Some("table:public.user_bonus") }));
    assert!(g.edges.iter().any(|e| {
        e.kind == "sql_reads" && e.to.as_deref() == Some("table:public.game_session")
    }));
}

#[test]
fn php_cte_delete_skips_cte_name() {
    let g = analyze_php(
        r#"<?php
namespace App;
class Cleaner {
    function run($db) {
        $db->createCommand("
            WITH cte AS (
                SELECT id FROM user_message WHERE created_at < :cutoff LIMIT :batch
            )
            DELETE FROM user_message WHERE id IN (SELECT id FROM cte)
        ")->execute();
    }
}
"#,
    );
    assert!(ids("table", &g).contains(&"table:public.user_message"));
    assert!(!ids("table", &g).iter().any(|id| id.contains("cte")));
    assert!(g.edges.iter().any(|e| e.kind == "sql_writes"));
}

#[test]
fn comments_and_strings_do_not_invent_declarations() {
    let g = analyze_sql(
        r#"
-- CREATE TABLE fake (id int);
/* CREATE TABLE also_fake (id int); */
CREATE TABLE real (id int, note text DEFAULT 'CREATE TABLE nested');
"#,
    );
    assert_eq!(ids("table", &g), ["table:public.real"]);
}

#[test]
fn syntax_errors_keep_other_statements() {
    let g = analyze_sql(
        r#"
CREATE TABLE ok (id int);
CREATE TABLE broken (;
CREATE TABLE also_ok (name text);
"#,
    );
    assert!(ids("table", &g).contains(&"table:public.ok"));
    assert!(ids("table", &g).contains(&"table:public.also_ok"));
    assert!(g
        .diagnostics
        .iter()
        .any(|d| d.code == "sql_statement_error"));
}

#[test]
fn declaration_paths_skip_vendor_sql_ddl() {
    let files = vec![
        SourceFile {
            path: "vendor/yiisoft/yii2/i18n/migrations/schema-sqlite.sql".into(),
            source: "CREATE INDEX idx_message_language ON message (language);\n".into(),
        },
        SourceFile {
            path: "schema.sql".into(),
            source: "CREATE TABLE wallet (id int);\n".into(),
        },
    ];
    let options = AnalysisOptions {
        declaration_paths: vec!["vendor/yiisoft".into()],
        ..Default::default()
    };
    let g = code_atlas::plugins::registry()
        .analyze(&files, &["pgsql".into()], &[], &options)
        .unwrap();
    assert!(
        !ids("index", &g)
            .iter()
            .any(|id| id.contains("idx_message_language")),
        "vendor declaration_paths SQL must not enter the graph"
    );
    assert!(ids("table", &g).contains(&"table:public.wallet"));
}

#[test]
fn php_and_pgsql_share_table_ids_without_duplicates() {
    // Slotor enables languages: [php, pgsql]. Embedded SQL stubs and .sql
    // CREATE TABLE both emit table:/schema: IDs — the registry must keep one.
    let files = vec![
        SourceFile {
            path: "Wallet.php".into(),
            source: r#"<?php
namespace App;
class WalletService {
    function debit($db, $amount, $id) {
        return $db->createCommand(
            'UPDATE wallet SET amount = amount - :amount WHERE id = :id',
            [':amount' => $amount, ':id' => $id]
        )->execute();
    }
}
"#
            .into(),
        },
        SourceFile {
            path: "schema.sql".into(),
            source: "CREATE TABLE wallet (id int PRIMARY KEY, amount numeric);\n".into(),
        },
    ];
    let g = code_atlas::plugins::registry()
        .analyze(
            &files,
            &["php".into(), "pgsql".into()],
            &[],
            &AnalysisOptions::default(),
        )
        .unwrap();
    let mut seen = std::collections::BTreeSet::new();
    let mut dupes = Vec::new();
    for node in &g.nodes {
        if !seen.insert(node.id.clone()) {
            dupes.push(node.id.clone());
        }
    }
    assert!(dupes.is_empty(), "duplicate node IDs: {dupes:?}");
    assert!(ids("table", &g).contains(&"table:public.wallet"));
    assert!(
        g.nodes
            .iter()
            .any(|n| n.id == "table:public.wallet" && n.span.path.ends_with(".sql")),
        "pgsql declaration should win over PHP stub"
    );
    let mut edge_seen = std::collections::BTreeSet::new();
    let mut edge_dupes = Vec::new();
    for edge in &g.edges {
        if !edge_seen.insert(edge.id.clone()) {
            edge_dupes.push(edge.id.clone());
        }
    }
    assert!(edge_dupes.is_empty(), "duplicate edge IDs: {edge_dupes:?}");
}

#[test]
fn plugin_registered_and_limits_enforced() {
    let catalog = code_atlas::plugins::registry().catalog();
    let languages: Vec<&str> = catalog["languages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();
    assert!(languages.contains(&"pgsql"));
    let mut tiny = AnalysisOptions {
        max_graph_elements: 1,
        ..Default::default()
    };
    tiny.deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    assert!(PgsqlLanguagePlugin
        .analyze(
            &[SourceFile {
                path: "a.sql".into(),
                source: "CREATE TABLE t(id int);".into(),
            }],
            &tiny
        )
        .is_err());
}
