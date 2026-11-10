//! Explicit integration target: run only against a disposable PostgreSQL database.
use code_atlas::{
    domain::*, postgres::PostgresRepository, repository::Repository, service::AtlasService,
};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

fn connection() -> PostgresRepository {
    let url = std::env::var("ATLAS_TEST_DATABASE_URL")
        .expect("set ATLAS_TEST_DATABASE_URL to a disposable database");
    let mut repository = PostgresRepository::connect(&url).unwrap();
    repository.migrate().unwrap();
    repository
}
fn unique() -> String {
    format!(
        "pg-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}
fn node(id: &str, path: &str) -> Node {
    Node {
        id: id.into(),
        kind: "method".into(),
        name: "value'; DROP TABLE atlas_nodes; --".into(),
        qualified_name: format!("Fixture::{id}"),
        span: SourceSpan {
            path: path.into(),
            start_line: 1,
            end_line: 3,
        },
        metadata: json!({"type":"int","text":"quoted ' and Unicode ש"}),
        trust: 1,
        revision: 1,
    }
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_real_constraints_roll_back_partial_updates() {
    let mut r = connection();
    let id = unique();
    let n = node(&id, &format!("{id}.php"));
    r.begin().unwrap();
    r.put_node(&n).unwrap();
    let invalid = Edge {
        id: format!("{id}-bad"),
        kind: "calls".into(),
        from: "does-not-exist".into(),
        to: Some(id.clone()),
        span: n.span.clone(),
        metadata: json!({}),
        trust: 1,
        revision: 1,
    };
    assert_eq!(r.put_edge(&invalid).unwrap_err().kind, ErrorKind::Storage);
    r.rollback().unwrap();
    assert!(r.node(&id).unwrap().is_none());
    r.begin().unwrap();
    r.put_node(&n).unwrap();
    r.commit().unwrap();
    assert_eq!(r.node(&id).unwrap().unwrap().name, n.name);
    r.begin().unwrap();
    r.delete_node(&id).unwrap();
    r.commit().unwrap();
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_cross_connection_revisions_review_and_analytics_round_trip() {
    let mut first = AtlasService::new(Box::new(connection()));
    let mut second = AtlasService::new(Box::new(connection()));
    let id = unique();
    let path = format!("{id}.php");
    first
        .index(
            GraphSnapshot {
                nodes: vec![node(&id, &path)],
                edges: vec![],
                diagnostics: vec![],
            },
            vec![path.clone()],
            vec![],
        )
        .unwrap();
    let revision = second.context(&id, 0, 10).unwrap().nodes[0].revision;
    let c = first
        .correct(
            "node",
            &id,
            revision,
            json!({"metadata":{"type":"string"}}),
            "SQL test",
        )
        .unwrap();
    assert_eq!(
        second
            .correct(
                "node",
                &id,
                revision,
                json!({"metadata":{"type":"bool"}}),
                "stale"
            )
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    let n = second.context(&id, 0, 10).unwrap().nodes.remove(0);
    assert_eq!(n.trust, 2);
    assert_eq!(n.metadata["type"], "string");
    second.review(&c.id, n.revision, true).unwrap();
    let n = first.context(&id, 0, 10).unwrap().nodes.remove(0);
    assert_eq!(n.trust, 3);
    assert_eq!(
        first
            .correct(
                "node",
                &id,
                n.revision,
                json!({"metadata":{"type":"bool"}}),
                "blocked"
            )
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    first
        .record_delivery(&id, &[id.clone(), id.clone()], &[], timestamp)
        .unwrap();
    second
        .record_delivery(&id, std::slice::from_ref(&id), &[], timestamp)
        .unwrap();
    let analytics = first.analytics(timestamp, timestamp + 1, 86400).unwrap();
    assert_eq!(analytics.total_responses, 1);
    assert_eq!(analytics.elements.len(), 1);
    first
        .index(GraphSnapshot::default(), vec![], vec![path.clone()])
        .unwrap();
    assert!(second.context(&id, 0, 10).is_err());
    let analytics = second.analytics(timestamp, timestamp + 1, 86400).unwrap();
    assert_eq!(analytics.elements[0].path, path);
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_parameterization_and_serde_round_trip() {
    let mut r = connection();
    let id = format!("{}-' OR 1=1 --", unique());
    let n = node(&id, &format!("{id}.php"));
    r.begin().unwrap();
    r.put_node(&n).unwrap();
    r.commit().unwrap();
    let actual = r.node(&id).unwrap().unwrap();
    assert_eq!(actual, n);
    assert!(r.node("' OR 1=1 --").unwrap().is_none());
    assert!(r
        .search_nodes(&NodeSearch {
            text: "DROP TABLE".into(),
            limit: 10,
            ..NodeSearch::default()
        })
        .unwrap()
        .iter()
        .any(|n| n.id == id));
    assert_eq!(r.node("bad\0id").unwrap_err().kind, ErrorKind::InvalidInput);
    r.begin().unwrap();
    r.delete_node(&id).unwrap();
    r.commit().unwrap();
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_search_filters_kinds_and_walks_keyset_cursor() {
    let mut r = connection();
    let prefix = unique();
    let marker = format!("{prefix}_dep_tax_control");
    let mut stored = vec![];
    for (i, kind) in [
        "write_site",
        "read_site",
        "write_site",
        "read_site",
        "write_site",
    ]
    .into_iter()
    .enumerate()
    {
        let mut n = node(&format!("{prefix}-{i}"), &format!("{prefix}.php"));
        n.kind = kind.into();
        n.name = format!("$wallet->{marker}");
        stored.push(n);
    }
    r.begin().unwrap();
    r.put_nodes(&stored).unwrap();
    r.commit().unwrap();
    let mut search = NodeSearch {
        text: marker.clone(),
        kinds: vec!["write_site".into()],
        after: None,
        limit: 2,
        ..NodeSearch::default()
    };
    let first = r.search_nodes(&search).unwrap();
    assert_eq!(
        first.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        [format!("{prefix}-0"), format!("{prefix}-2")]
    );
    search.after = Some(first[1].id.clone());
    let second = r.search_nodes(&search).unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].id, format!("{prefix}-4"));
    search.kinds = vec![];
    search.after = None;
    search.limit = 10;
    assert_eq!(r.search_nodes(&search).unwrap().len(), 5);
    r.begin().unwrap();
    r.delete_nodes(&stored.iter().map(|n| n.id.clone()).collect::<Vec<_>>())
        .unwrap();
    r.commit().unwrap();
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_symbol_search_and_index_metadata_round_trip() {
    let mut r = connection();
    let prefix = unique();
    let member = format!("{}_run", prefix.replace('-', "_"));
    let mut call = node(&format!("{prefix}-call"), &format!("{prefix}.php"));
    call.kind = "call_site".into();
    call.name = format!("$a->{member}()");
    call.metadata = json!({"member_name": member});
    let mut log = node(&format!("{prefix}-log"), &format!("{prefix}.php"));
    log.kind = "call_site".into();
    log.name = format!("Logger::info('{member}')");
    log.metadata = json!({"member_name": "info"});
    r.begin().unwrap();
    r.put_nodes(&[call.clone(), log.clone()]).unwrap();
    r.set_meta("indexed_at", "1700000000").unwrap();
    r.commit().unwrap();
    let mut search = NodeSearch {
        text: member.to_uppercase(),
        mode: SearchMode::Symbol,
        limit: 10,
        ..NodeSearch::default()
    };
    let found = r.search_nodes(&search).unwrap();
    assert_eq!(
        found.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        [call.id.as_str()]
    );
    // Text mode ignores the name inside the log string literal; substring keeps it.
    search.mode = SearchMode::Text;
    search.text = member.clone();
    assert_eq!(r.search_nodes(&search).unwrap().len(), 1);
    search.mode = SearchMode::Substring;
    assert_eq!(r.search_nodes(&search).unwrap().len(), 2);
    let mut other = connection();
    assert_eq!(
        other.meta("indexed_at").unwrap().as_deref(),
        Some("1700000000")
    );
    r.begin().unwrap();
    r.delete_nodes(&[call.id, log.id]).unwrap();
    r.commit().unwrap();
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_reset_clears_every_atlas_table() {
    let mut r = connection();
    let id = unique();
    r.begin().unwrap();
    r.put_node(&node(&id, &format!("{id}.php"))).unwrap();
    r.set_meta("indexed_at", "1").unwrap();
    r.commit().unwrap();
    r.begin().unwrap();
    r.reset().unwrap();
    r.commit().unwrap();
    assert!(r.nodes().unwrap().is_empty());
    assert!(r.edges().unwrap().is_empty());
    assert!(r.corrections().unwrap().is_empty());
    assert!(r.diagnostics().unwrap().is_empty());
    assert!(r.deliveries(0, i64::MAX).unwrap().is_empty());
    assert_eq!(r.meta("indexed_at").unwrap(), None);
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_existing_node_ids_checks_many_ids_in_one_query() {
    let mut r = connection();
    let id = format!("{}-\\\\x,'\"", unique());
    r.begin().unwrap();
    r.put_node(&node(&id, &format!("{}.php", unique())))
        .unwrap();
    r.commit().unwrap();
    let asked: std::collections::BTreeSet<String> = [id.clone(), "missing-node".to_owned()]
        .into_iter()
        .collect();
    let found = r.existing_node_ids(&asked).unwrap();
    assert_eq!(
        found.into_iter().collect::<Vec<_>>(),
        std::slice::from_ref(&id)
    );
    assert!(r.existing_node_ids(&Default::default()).unwrap().is_empty());
    r.begin().unwrap();
    r.delete_node(&id).unwrap();
    r.commit().unwrap();
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_text_search_matches_whole_identifiers_and_substring_keeps_partial() {
    let mut r = connection();
    let tag = unique().replace('-', "_");
    let member = format!("get{tag}Bonus");
    let site = |suffix: &str, kind: &str, name: String, metadata: serde_json::Value| {
        let mut n = node(&format!("{tag}-{suffix}"), &format!("{tag}.php"));
        n.kind = kind.into();
        n.name = name;
        n.qualified_name = String::new();
        n.metadata = metadata;
        n
    };
    let nodes = vec![
        site("decl", "method", member.clone(), json!({})),
        site("longer", "method", format!("{member}Cod"), json!({})),
        site(
            "call",
            "call_site",
            format!("$this->{member}()"),
            json!({"expression": format!("$this->{member}()")}),
        ),
        site(
            "glued",
            "read_site",
            format!("$tar{member}"),
            json!({"expression": format!("$tar{member}")}),
        ),
        site(
            "log",
            "call_site",
            format!("Logger::info('{member} failed')"),
            json!({"expression": format!("Logger::info('{member} failed')"), "member_name": "info"}),
        ),
        {
            let mut this = site("this", "variable", "$this".into(), json!({}));
            this.qualified_name = format!("method:A::{member}:$this");
            this
        },
    ];
    r.begin().unwrap();
    r.put_nodes(&nodes).unwrap();
    r.commit().unwrap();
    let ids = |r: &mut PostgresRepository, text: &str, mode: SearchMode| {
        let mut ids: Vec<String> = r
            .search_nodes(&NodeSearch {
                text: text.into(),
                limit: 50,
                mode,
                ..NodeSearch::default()
            })
            .unwrap()
            .into_iter()
            .map(|n| n.id)
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(
        ids(&mut r, &member, SearchMode::Text),
        [format!("{tag}-call"), format!("{tag}-decl")]
    );
    assert_eq!(
        ids(&mut r, &format!("->{member}("), SearchMode::Text),
        [format!("{tag}-call")]
    );
    assert_eq!(ids(&mut r, &member, SearchMode::Substring).len(), 5);
    r.begin().unwrap();
    r.delete_nodes(&nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>())
        .unwrap();
    r.commit().unwrap();
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_search_excludes_paths_by_regex() {
    let mut r = connection();
    let tag = unique().replace('-', "_");
    let mut production = node(&format!("{tag}-prod"), &format!("app/{tag}.php"));
    production.name = format!("run{tag}");
    let mut test = node(
        &format!("{tag}-test"),
        &format!("common/tests/{tag}Test.php"),
    );
    test.name = format!("run{tag}");
    r.begin().unwrap();
    r.put_nodes(&[production.clone(), test.clone()]).unwrap();
    r.commit().unwrap();
    let ids = |r: &mut PostgresRepository, exclude_paths: Option<String>| {
        let mut ids: Vec<String> = r
            .search_nodes(&NodeSearch {
                text: format!("run{tag}"),
                limit: 10,
                exclude_paths,
                ..NodeSearch::default()
            })
            .unwrap()
            .into_iter()
            .map(|n| n.id)
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(&mut r, None).len(), 2);
    assert_eq!(
        ids(&mut r, code_atlas::glob::any_regex(&["*/tests/*".into()])),
        [production.id.clone()]
    );
    r.begin().unwrap();
    r.delete_nodes(&[production.id, test.id]).unwrap();
    r.commit().unwrap();
}

#[test]
#[ignore = "requires ATLAS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
fn postgres_search_keeps_only_included_paths() {
    let mut r = connection();
    let tag = unique().replace('-', "_");
    let mut partner = node(&format!("{tag}-p"), &format!("partner/{tag}.php"));
    partner.name = format!("run{tag}");
    let mut admin = node(&format!("{tag}-a"), &format!("admin/{tag}.php"));
    admin.name = format!("run{tag}");
    r.begin().unwrap();
    r.put_nodes(&[partner.clone(), admin.clone()]).unwrap();
    r.commit().unwrap();
    let found: Vec<String> = r
        .search_nodes(&NodeSearch {
            text: format!("run{tag}"),
            limit: 10,
            include_paths: Some("^(?:admin)(?:/|$)".into()),
            ..NodeSearch::default()
        })
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(found, [admin.id.clone()]);
    r.begin().unwrap();
    r.delete_nodes(&[partner.id, admin.id]).unwrap();
    r.commit().unwrap();
}
