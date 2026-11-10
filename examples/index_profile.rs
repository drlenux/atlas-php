//! Development-only indexing profile; publication uses MemoryRepository, not PostgreSQL.
use code_atlas::{
    config::Config, indexer::IndexCoordinator, repository::MemoryRepository, service::AtlasService,
};
use std::{collections::BTreeMap, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .ok_or("usage: index_profile PROJECT_ROOT WORKERS")?;
    let workers = args
        .next()
        .ok_or("usage: index_profile PROJECT_ROOT WORKERS")?;
    let config = Config::from_map(&BTreeMap::from([
        ("ATLAS_PROJECT_ROOT".into(), root),
        ("ATLAS_INDEX_WORKERS".into(), workers),
        ("ATLAS_DATABASE_URL".into(), "dbname=unused".into()),
        ("ATLAS_AGENT_TOKEN".into(), "profile-agent".into()),
        ("ATLAS_REVIEW_TOKEN".into(), "profile-reviewer".into()),
    ]))?;
    let workers = config.index_workers;
    let mut service = AtlasService::new(Box::new(MemoryRepository::new()));
    let start = Instant::now();
    let report = IndexCoordinator::new(config).index(&mut service)?;
    println!(
        "{}",
        serde_json::json!({"workers": workers, "elapsed_ms": start.elapsed().as_millis(), "report": report})
    );
    Ok(())
}
