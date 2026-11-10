use code_atlas::{
    config::Config,
    http::{self, HttpConfig, RpcReply},
    indexer::IndexCoordinator,
    postgres::PostgresRepository,
    project::{discover, pending_paths, Debouncer},
    service::AtlasService,
};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

fn main() {
    if let Err(error) = execute() {
        eprintln!("code-atlas: {error}");
        std::process::exit(1);
    }
}

fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    if mode == "--help" || mode == "help" {
        println!("code-atlas [serve|index|migrate|stdio|plugins|healthcheck]\nConfiguration: ATLAS_* environment variables; see .env.example and README.md");
        return Ok(());
    }
    if mode == "plugins" {
        println!("{}", code_atlas::plugins::registry().catalog());
        return Ok(());
    }
    if mode == "healthcheck" {
        return healthcheck();
    }
    if !["serve", "index", "migrate", "stdio"].contains(&mode.as_str()) {
        return Err("unknown command; use --help".into());
    }
    let config = Config::from_env()?;
    let mut repository = PostgresRepository::connect(&config.database_url)?;
    repository.migrate()?;
    let mut service = AtlasService::new(Box::new(repository));
    if mode == "migrate" {
        eprintln!("schema ready");
        return Ok(());
    }
    if mode == "index" {
        let report = IndexCoordinator::new(config).index(&mut service)?;
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    let http_config = HttpConfig {
        agent_token: config.agent_token.clone(),
        review_token: config.review_token.clone(),
        allowed_origins: config.allowed_origins.clone(),
        max_request_bytes: config.max_request_bytes,
        max_context_nodes: config.max_context_nodes,
        max_context_depth: config.max_context_depth,
        project_root: config.project_root.clone(),
        watch_dirs: config.watch_dirs.clone(),
        excludes: config.excludes.clone(),
        include_vendor: config.include_vendor,
        declaration_paths: config.declaration_paths.clone(),
        language_plugins: config.language_plugins.clone(),
        framework_plugins: config.framework_plugins.clone(),
        max_file_bytes: config.max_file_bytes,
        grep_max_file_bytes: config.grep_max_file_bytes,
        test_paths: config.test_paths.clone(),
        grep_excludes: config.grep_excludes.clone(),
        read_excludes: config.read_excludes.clone(),
        areas: config.areas.clone(),
    };
    if mode == "stdio" {
        return stdio(&mut service, &http_config);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.cpu_limit.ceil().clamp(1.0, 32.0) as usize)
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(config.bind).await?;
        eprintln!("code-atlas listening on {}", config.bind);
        let service = Arc::new(Mutex::new(service));
        let app = http::router(service.clone(), http_config);
        let stop = Arc::new(AtomicBool::new(false));
        spawn_watcher(service.clone(), config, stop.clone());
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown())
            .await;
        stop.store(true, Ordering::Relaxed);
        result
    })?;
    Ok(())
}

fn stdio(
    service: &mut AtlasService,
    config: &HttpConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // Stdio attaches to an existing indexed PostgreSQL atlas. It does not start another watcher.
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    loop {
        let mut line = Vec::new();
        let read = std::io::Read::take(&mut reader, config.max_request_bytes as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > config.max_request_bytes {
            return Err("MCP message exceeds ATLAS_MAX_REQUEST_BYTES".into());
        }
        let response = match serde_json::from_slice(&line) {
            Ok(request) => http::dispatch_rpc(service, request, config),
            Err(_) => RpcReply::Response(
                serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
        };
        if let RpcReply::Response(response) = response {
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn spawn_watcher(service: Arc<Mutex<AtlasService>>, config: Config, stop: Arc<AtomicBool>) {
    let progress = service
        .lock()
        .expect("service available before watcher starts")
        .indexing_progress();
    // The observer scans files independently of builds, so pending changes are
    // reported while a build runs; the builder consumes its latest scan.
    let latest: Arc<Mutex<Option<BTreeMap<String, u64>>>> = Arc::default();
    let published: Arc<Mutex<Option<BTreeMap<String, u64>>>> = Arc::default();
    spawn_observer(
        config.clone(),
        stop.clone(),
        progress.clone(),
        latest.clone(),
        published.clone(),
    );
    thread::spawn(move || {
        let mut coordinator = IndexCoordinator::new(config.clone()).with_progress(progress.clone());
        let mut debounce = Debouncer::new(config.debounce_ms);
        let started = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            let attempt = (|| -> Result<(), String> {
                let Some(signature) = latest.lock().map_err(|_| "scan lock unavailable")?.clone()
                else {
                    return Ok(());
                };
                let changed = debounce.observe(&signature, started.elapsed().as_millis() as u64);
                // A reviewer's manual request (or a reset) runs a full rebuild.
                let full = progress.take_reindex_request();
                if full || changed {
                    // Analysis runs outside the service lock; only publication is serialized.
                    let prepared = if full {
                        coordinator.prepare()?
                    } else {
                        coordinator.prepare_changes()?
                    };
                    let mut service = service.lock().map_err(|_| "service lock unavailable")?;
                    let report = coordinator.publish(&mut service, prepared)?;
                    drop(service);
                    let files = coordinator.published_files().cloned();
                    if let Some(files) = &files {
                        progress.set_pending_paths(pending_paths(files, &signature));
                    }
                    *published
                        .lock()
                        .map_err(|_| "publication lock unavailable")? = files;
                    eprintln!(
                        "indexed ({}): {} nodes, {} edges, {} diagnostics",
                        coordinator
                            .last_scope()
                            .map_or("full".to_owned(), |scope| format!("{} files", scope.len())),
                        report.nodes,
                        report.edges,
                        report.diagnostics
                    );
                }
                Ok(())
            })();
            if let Err(error) = attempt {
                if progress.snapshot().state != "failed" {
                    if progress.snapshot().state != "running" {
                        progress.start(config.poll_ms);
                    }
                    progress.fail(&error);
                }
                eprintln!("indexing: {error}");
                debounce.retry_after(started.elapsed().as_millis() as u64);
            }
            thread::sleep(Duration::from_millis(config.poll_ms));
        }
    });
}

fn spawn_observer(
    config: Config,
    stop: Arc<AtomicBool>,
    progress: code_atlas::progress::IndexingProgress,
    latest: Arc<Mutex<Option<BTreeMap<String, u64>>>>,
    published: Arc<Mutex<Option<BTreeMap<String, u64>>>>,
) {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match discover(&config) {
                Ok(scan) => {
                    if let Some(files) = published.lock().ok().and_then(|p| p.clone()) {
                        progress.set_pending_paths(pending_paths(&files, &scan.files));
                    }
                    if let Ok(mut latest) = latest.lock() {
                        *latest = Some(scan.files);
                    }
                }
                Err(error) => eprintln!("watching: {error}"),
            }
            thread::sleep(Duration::from_millis(config.poll_ms));
        }
    });
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn healthcheck() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        io::Read,
        net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    };
    let mut address: SocketAddr = std::env::var("ATLAS_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    if address.ip().is_unspecified() {
        address.set_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.take(1024).read_to_string(&mut response)?;
    if !response.starts_with("HTTP/1.1 200 ") {
        return Err("health endpoint did not return 200".into());
    }
    Ok(())
}
