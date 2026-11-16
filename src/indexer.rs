use crate::{
    analyzer::{
        incremental::{declaration_surface, Surface},
        AnalysisOptions, Incremental, PluginRegistry, SourceFile,
    },
    config::Config,
    diagnostics_log,
    domain::{Diagnostic, GraphSnapshot, IndexReport},
    logging::AtlasLog,
    progress::IndexingProgress,
    project::discover,
    service::AtlasService,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufWriter, Read, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct IndexCoordinator {
    pub config: Config,
    plugins: PluginRegistry,
    progress: IndexingProgress,
    log: AtlasLog,
    /// What the last successful publication was built from (incremental baseline).
    published: Option<Published>,
    last_scope: Option<BTreeSet<String>>,
}
struct Published {
    files: BTreeMap<String, u64>,
    surface: Surface,
}
pub struct PreparedIndex {
    snapshot: GraphSnapshot,
    /// When set, nodes/edges were spilled to disk so publish does not keep a second peak.
    spill_dir: Option<PathBuf>,
    indexed_paths: Vec<String>,
    observed: BTreeMap<String, u64>,
    surface: Surface,
    /// `Some` for an incremental build: the only paths it publishes.
    scope: Option<BTreeSet<String>>,
}
impl IndexCoordinator {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            plugins: crate::plugins::registry(),
            progress: IndexingProgress::default(),
            // File log comes from `with_log` in serve/index (one open). Tests stay silent.
            log: AtlasLog::default(),
            published: None,
            last_scope: None,
        }
    }
    pub fn with_plugins(config: Config, plugins: PluginRegistry) -> Self {
        Self {
            config,
            plugins,
            progress: IndexingProgress::default(),
            log: AtlasLog::default(),
            published: None,
            last_scope: None,
        }
    }
    pub fn with_log(mut self, log: AtlasLog) -> Self {
        self.log = log;
        self
    }
    pub fn with_progress(mut self, progress: IndexingProgress) -> Self {
        self.progress = progress;
        self
    }
    /// Full rebuild (startup, manual reindex, after a reset).
    pub fn index(&mut self, service: &mut AtlasService) -> Result<IndexReport, String> {
        self.progress = service.indexing_progress();
        let prepared = self.prepare()?;
        trim_allocator();
        self.publish(service, prepared)
    }
    /// Rebuild of changed files and their dependents; full when there is no baseline.
    pub fn index_changes(&mut self, service: &mut AtlasService) -> Result<IndexReport, String> {
        self.progress = service.indexing_progress();
        let prepared = self.prepare_changes()?;
        trim_allocator();
        self.publish(service, prepared)
    }
    /// File hashes of the last successful publication.
    pub fn published_files(&self) -> Option<&BTreeMap<String, u64>> {
        self.published.as_ref().map(|published| &published.files)
    }
    /// Paths of the last publication when it was incremental; `None` after a full one.
    pub fn last_scope(&self) -> Option<&BTreeSet<String>> {
        self.last_scope.as_ref()
    }
    pub fn prepare(&self) -> Result<PreparedIndex, String> {
        self.prepare_with(false)
    }
    pub fn prepare_changes(&self) -> Result<PreparedIndex, String> {
        self.prepare_with(true)
    }
    fn prepare_with(&self, incremental: bool) -> Result<PreparedIndex, String> {
        self.progress.start(self.config.poll_ms);
        let result = self.prepare_snapshot(incremental);
        if let Err(error) = &result {
            self.progress.fail(error);
        }
        result
    }
    fn prepare_snapshot(&self, incremental: bool) -> Result<PreparedIndex, String> {
        let before = discover(&self.config)?;
        let context = self
            .published
            .as_ref()
            .filter(|_| incremental)
            .map(|published| {
                let changed = before
                    .files
                    .iter()
                    .filter(|(path, hash)| published.files.get(*path) != Some(hash))
                    .map(|(path, _)| path.clone())
                    .collect();
                let removed = published
                    .files
                    .keys()
                    .filter(|path| !before.files.contains_key(*path))
                    .cloned()
                    .collect();
                std::sync::Arc::new(Incremental::new(
                    changed,
                    removed,
                    published.surface.clone(),
                ))
            });
        let options = AnalysisOptions {
            language_versions: self.config.language_versions.clone(),
            max_graph_elements: self.config.max_graph_elements,
            deadline: Instant::now() + Duration::from_secs(self.config.analysis_timeout_secs),
            workers: self.config.index_workers,
            progress: self.progress.clone(),
            // With vendor fully included, its files get full analysis.
            declaration_paths: if self.config.include_vendor {
                Vec::new()
            } else {
                self.config.declaration_paths.clone()
            },
            incremental: context.clone(),
            areas: self.config.areas.clone(),
            log: self.log.clone(),
        };
        let mut diagnostics = Vec::new();
        let mut files = Vec::new();
        for warning in &before.diagnostics {
            let path = warning
                .split_once(": ")
                .map(|v| v.0)
                .unwrap_or("")
                .to_owned();
            diagnostics.push(Diagnostic {
                path,
                line: 0,
                code: "file_limit".into(),
                message: warning.clone(),
            });
        }
        let paths: Vec<String> = before.files.keys().cloned().collect();
        let mut loaded: Vec<Option<Result<SourceFile, Diagnostic>>> =
            paths.iter().map(|_| None).collect();
        let total_bytes = AtomicU64::new(0);
        let max_file_bytes = self.config.max_file_bytes;
        let max_analysis_bytes = self.config.max_analysis_bytes;
        let project_root = self.config.project_root.clone();
        self.progress
            .phase("reading", "sources", Some(paths.len()), "files");
        self.progress
            .set_workers(options.workers.min(paths.len().max(1)));
        crate::parallel::try_for_each_mut(
            &mut loaded,
            options.workers.min(paths.len().max(1)).max(1),
            || (),
            |_, position, slot| {
                let path = &paths[position];
                self.progress.work_started(path);
                options.check(0)?;
                let full_path = project_root.join(path);
                let result = (|| -> Result<SourceFile, Diagnostic> {
                    let loaded = (|| -> Result<String, String> {
                        let metadata =
                            fs::symlink_metadata(&full_path).map_err(|e| e.to_string())?;
                        if metadata.file_type().is_symlink()
                            || !full_path
                                .canonicalize()
                                .map_err(|e| e.to_string())?
                                .starts_with(&project_root)
                        {
                            return Err("source escaped project or became a symbolic link".into());
                        }
                        let mut bytes = Vec::new();
                        fs::File::open(&full_path)
                            .map_err(|e| e.to_string())?
                            .take(max_file_bytes + 1)
                            .read_to_end(&mut bytes)
                            .map_err(|e| e.to_string())?;
                        if bytes.len() as u64 > max_file_bytes {
                            return Err("file exceeds ATLAS_MAX_FILE_BYTES".into());
                        }
                        String::from_utf8(bytes).map_err(|_| "source is not valid UTF-8".into())
                    })();
                    match loaded {
                        Ok(source) => {
                            let used = total_bytes.fetch_add(source.len() as u64, Ordering::SeqCst)
                                + source.len() as u64;
                            if used > max_analysis_bytes {
                                return Err(Diagnostic {
                                    path: path.clone(),
                                    line: 0,
                                    code: "read_error".into(),
                                    message: "input exceeds ATLAS_MAX_ANALYSIS_BYTES; narrow the watched area or raise the limit".into(),
                                });
                            }
                            Ok(SourceFile {
                                path: path.clone(),
                                source,
                            })
                        }
                        Err(message) => Err(Diagnostic {
                            path: path.clone(),
                            line: 0,
                            code: "read_error".into(),
                            message,
                        }),
                    }
                })();
                self.progress.work_finished(path);
                if let Err(diagnostic) = &result {
                    if diagnostic.message.contains("ATLAS_MAX_ANALYSIS_BYTES") {
                        return Err(diagnostic.message.clone());
                    }
                }
                *slot = Some(result);
                Ok(())
            },
        )?;
        let mut sources = Vec::new();
        for slot in loaded {
            match slot.expect("reading worker filled its result slot") {
                Ok(source) => {
                    files.push(source.path.clone());
                    sources.push(source);
                }
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }
        let mut snapshot = self.plugins.analyze(
            &sources,
            &self.config.language_plugins,
            &self.config.framework_plugins,
            &options,
        )?;
        // Analysis copied what it needs; free source texts before publish peak.
        drop(sources);
        snapshot.diagnostics.extend(diagnostics);
        let surface = declaration_surface(&snapshot.nodes);
        let scope = context.and_then(|context| context.scope());
        if let Some(scope) = &scope {
            // Unchanged files keep their published facts; publish only the scope.
            snapshot.nodes.retain(|n| scope.contains(&n.span.path));
            snapshot.edges.retain(|e| scope.contains(&e.span.path));
            snapshot.diagnostics.retain(|d| scope.contains(&d.path));
        }
        let failed: BTreeSet<_> = snapshot
            .diagnostics
            .iter()
            .filter(|d| {
                matches!(
                    d.code.as_str(),
                    "parse_error"
                        | "file_unavailable"
                        | "path_outside_root"
                        | "unsupported_syntax"
                        | "read_error"
                        | "file_limit"
                )
            })
            .map(|d| d.path.clone())
            .collect();
        // Nodes from a failed file must not remain: apply_snapshot requires every
        // node path to be in indexed_paths (successful scope).
        if !failed.is_empty() {
            snapshot.nodes.retain(|n| !failed.contains(&n.span.path));
            snapshot.edges.retain(|e| !failed.contains(&e.span.path));
        }
        let indexed_paths: Vec<_> = files
            .into_iter()
            .filter(|path| !failed.contains(path))
            .filter(|path| scope.as_ref().is_none_or(|scope| scope.contains(path)))
            .collect();
        self.progress
            .phase("validation", "after_analysis", None, "files");
        if discover(&self.config)?.files != before.files {
            return Err("source changed during analysis; snapshot not published".into());
        }
        // Large graphs: spill nodes/edges before publish so RSS can fall after analysis.
        let spill_dir = if snapshot.nodes.len() >= SPILL_NODE_THRESHOLD {
            self.progress.phase(
                "publishing",
                "spill_snapshot",
                Some(snapshot.nodes.len()),
                "items",
            );
            Some(spill_snapshot(&mut snapshot)?)
        } else {
            None
        };
        trim_allocator();
        Ok(PreparedIndex {
            snapshot,
            spill_dir,
            indexed_paths,
            observed: before.files,
            surface,
            scope,
        })
    }
    pub fn publish(
        &mut self,
        service: &mut AtlasService,
        mut prepared: PreparedIndex,
    ) -> Result<IndexReport, String> {
        let baseline = Published {
            files: prepared.observed.clone(),
            surface: std::mem::take(&mut prepared.surface),
        };
        let scope = prepared.scope.clone();
        let spill_dir = prepared.spill_dir.clone();
        // Before commit: parse/unsupported/analysis gaps must be in docker logs / file.
        diagnostics_log::log_diagnostics(&prepared.snapshot.diagnostics, &self.log);
        let result = self.publish_snapshot(service, prepared);
        if let Some(dir) = spill_dir {
            let _ = fs::remove_dir_all(dir);
        }
        match &result {
            Ok(_) => {
                self.published = Some(baseline);
                self.last_scope = scope;
                self.progress.complete();
                trim_allocator();
            }
            Err(error) => self.progress.fail(error),
        }
        result
    }
    fn publish_snapshot(
        &self,
        service: &mut AtlasService,
        prepared: PreparedIndex,
    ) -> Result<IndexReport, String> {
        self.progress
            .phase("validation", "before_commit", None, "files");
        if discover(&self.config)?.files != prepared.observed {
            return Err("source changed before commit; snapshot not published".into());
        }
        self.progress
            .phase("publishing", "transaction", None, "items");
        let known_paths = service.source_paths().map_err(|e| e.to_string())?;
        let removed_paths=known_paths.into_iter().filter(|path| {
        // Only a filesystem NotFound confirms deletion; permission failures do not.
        matches!(std::fs::symlink_metadata(self.config.project_root.join(path)),Err(error) if error.kind()==std::io::ErrorKind::NotFound)
    }).collect();
        if let Some(dir) = &prepared.spill_dir {
            service
                .index_spilled(
                    dir,
                    prepared.snapshot.diagnostics,
                    prepared.indexed_paths,
                    removed_paths,
                )
                .map_err(|e| e.to_string())
        } else {
            service
                .index(prepared.snapshot, prepared.indexed_paths, removed_paths)
                .map_err(|e| e.to_string())
        }
    }
}

/// Spill graphs this large so publish does not keep analysis peak + write buffers.
const SPILL_NODE_THRESHOLD: usize = 50_000;

fn spill_snapshot(snapshot: &mut GraphSnapshot) -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("atlas-spill-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    {
        let mut out =
            BufWriter::new(fs::File::create(dir.join("nodes.jsonl")).map_err(|e| e.to_string())?);
        for node in &snapshot.nodes {
            serde_json::to_writer(&mut out, node).map_err(|e| e.to_string())?;
            out.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        out.flush().map_err(|e| e.to_string())?;
    }
    {
        let mut out =
            BufWriter::new(fs::File::create(dir.join("edges.jsonl")).map_err(|e| e.to_string())?);
        for edge in &snapshot.edges {
            serde_json::to_writer(&mut out, edge).map_err(|e| e.to_string())?;
            out.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        out.flush().map_err(|e| e.to_string())?;
    }
    snapshot.nodes.clear();
    snapshot.edges.clear();
    snapshot.nodes.shrink_to_fit();
    snapshot.edges.shrink_to_fit();
    Ok(dir)
}

/// Ask glibc to return free arenas to the OS after a full-index peak.
fn trim_allocator() {
    #[cfg(target_os = "linux")]
    unsafe {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        let _ = malloc_trim(0);
    }
}
