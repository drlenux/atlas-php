use crate::config::Config;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    hash::{DefaultHasher, Hasher},
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Default)]
pub struct Discovery {
    pub files: BTreeMap<String, u64>,
    pub diagnostics: Vec<String>,
}

pub fn discover(config: &Config) -> Result<Discovery, String> {
    if !config.project_root.is_dir() {
        return Err("project root unavailable".into());
    }
    let extensions = crate::plugins::registry()
        .extensions(&config.language_plugins, &config.framework_plugins)?;
    let mut scan = Discovery::default();
    for (relative, path, metadata) in iter_source_files(config, Some(&extensions))? {
        if metadata.len() > config.max_file_bytes {
            scan.diagnostics
                .push(format!("{relative}: file exceeds ATLAS_MAX_FILE_BYTES"));
            continue;
        }
        let mut file =
            fs::File::open(&path).map_err(|error| format!("cannot read {relative}: {error}"))?;
        let mut hasher = DefaultHasher::new();
        let mut buffer = [0_u8; 16_384];
        let mut bytes = 0;
        loop {
            let length = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if length == 0 {
                break;
            }
            bytes += length as u64;
            if bytes > config.max_file_bytes {
                return Err(format!("{relative}: file grew beyond size limit"));
            }
            hasher.write(&buffer[..length]);
        }
        scan.files.insert(relative, hasher.finish());
    }
    Ok(scan)
}

/// Every file under the watched tree (same exclusion rules as indexing, any extension).
/// Text search decides itself about size and binary content.
pub fn list_source_paths(config: &Config) -> Result<Vec<String>, String> {
    if !config.project_root.is_dir() {
        return Err("project root unavailable".into());
    }
    let mut paths: Vec<String> = iter_source_files(config, None)?
        .into_iter()
        .map(|(relative, _, _)| relative)
        .collect();
    paths.sort();
    Ok(paths)
}

/// Discovery knobs for withGrep without a full runtime [`Config`].
pub struct SourceWalk<'a> {
    pub project_root: &'a Path,
    pub watch_dirs: &'a [String],
    pub excludes: &'a [String],
    pub include_vendor: bool,
    pub declaration_paths: &'a [String],
    pub language_plugins: &'a [String],
    pub framework_plugins: &'a [String],
    pub max_file_bytes: u64,
}

impl SourceWalk<'_> {
    pub fn list_paths(&self) -> Result<Vec<String>, String> {
        let config = Config {
            database_url: String::new(),
            project_root: self.project_root.to_path_buf(),
            bind: "127.0.0.1:0".parse().expect("bind"),
            debounce_ms: 0,
            poll_ms: 1,
            watch_dirs: self.watch_dirs.to_vec(),
            excludes: self.excludes.to_vec(),
            grep_excludes: self.excludes.to_vec(),
            read_excludes: Vec::new(),
            areas: Vec::new(),
            include_vendor: self.include_vendor,
            declaration_paths: self.declaration_paths.to_vec(),
            language_plugins: self.language_plugins.to_vec(),
            framework_plugins: self.framework_plugins.to_vec(),
            language_versions: BTreeMap::new(),
            max_file_bytes: self.max_file_bytes,
            grep_max_file_bytes: self.max_file_bytes,
            test_paths: Vec::new(),
            max_analysis_bytes: self.max_file_bytes,
            max_graph_elements: 1,
            analysis_timeout_secs: 1,
            cpu_limit: 1.0,
            index_workers: 1,
            memory_limit_mb: 64,
            max_context_nodes: 1,
            max_context_depth: 0,
            max_request_bytes: 1024,
            agent_token: "x".into(),
            review_token: "y".into(),
            allowed_origins: vec![],
        };
        list_source_paths(&config)
    }
}

/// `extensions: None` lists every regular file (text search); `Some` only analyzer inputs.
fn iter_source_files(
    config: &Config,
    extensions: Option<&BTreeSet<String>>,
) -> Result<Vec<(String, PathBuf, fs::Metadata)>, String> {
    let mut files = Vec::new();
    let mut visited = HashSet::new();
    // Compare against the real root: on macOS /var → /private/var, so a
    // non-canonical project_root would make every directory look escaped.
    let root = config
        .project_root
        .canonicalize()
        .map_err(|error| format!("cannot resolve project root: {error}"))?;
    let mut pending: Vec<PathBuf> = config.watch_dirs.iter().map(|dir| root.join(dir)).collect();
    if config.include_vendor && root.join("vendor").is_dir() {
        pending.push(root.join("vendor"));
    } else {
        // Declaration-only library paths; a missing path is simply absent.
        pending.extend(config.declaration_paths.iter().map(|dir| root.join(dir)));
    }
    while let Some(path) = pending.pop() {
        let relative = path
            .strip_prefix(&root)
            .map_err(|_| "path escaped project")?
            .to_string_lossy()
            .replace('\\', "/");
        if excluded(&relative, config) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("cannot inspect {relative}: {error}")),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let canonical = path.canonicalize().map_err(|error| error.to_string())?;
            if !canonical.starts_with(&root) || !visited.insert(canonical) {
                continue;
            }
            for entry in
                fs::read_dir(&path).map_err(|error| format!("cannot read {relative}: {error}"))?
            {
                pending.push(entry.map_err(|error| error.to_string())?.path());
            }
        } else if metadata.is_file()
            && extensions.is_none_or(|extensions| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|extension| extensions.contains(&extension.to_ascii_lowercase()))
            })
        {
            files.push((relative, path, metadata));
        }
    }
    Ok(files)
}

/// Tool, VCS and IDE directories: never project source.
const INTERNAL: &[&str] = &[
    ".git",
    ".idea",
    ".vscode",
    ".agents",
    ".codex",
    ".claude",
    ".atlas",
    "node_modules",
    "target",
];

/// Tool/VCS/IDE directories and `.env` files: never indexed, searched or read.
pub fn always_excluded(relative: &str) -> bool {
    relative.split('/').any(|part| INTERNAL.contains(&part))
        || Path::new(relative)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == ".env" || name.starts_with(".env."))
}

/// `relative` falls under one of `patterns` (prefix, wildcard path or file-name pattern).
pub fn matches_exclusion(relative: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim_end_matches('/');
        relative == pattern
            || relative.starts_with(&format!("{pattern}/"))
            || wildcard(pattern.as_bytes(), relative.as_bytes())
            || (!pattern.contains('/')
                && Path::new(relative).file_name().is_some_and(|name| {
                    wildcard(pattern.as_bytes(), name.to_string_lossy().as_bytes())
                }))
    })
}

fn excluded(relative: &str, config: &Config) -> bool {
    if always_excluded(relative) {
        return true;
    }
    let declared = config.declaration_paths.iter().any(|prefix| {
        let prefix = prefix.trim_end_matches('/');
        relative == prefix || relative.starts_with(&format!("{prefix}/"))
    });
    if relative
        .split('/')
        .any(|part| part == "vendor" && !config.include_vendor && !declared)
    {
        return true;
    }
    matches_exclusion(relative, &config.excludes)
}

// '*' matches any sequence (including '/'); '?' matches one byte. No shell is used.
fn wildcard(pattern: &[u8], value: &[u8]) -> bool {
    let (mut p, mut v, mut star, mut matched) = (0, 0, None, 0);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            matched = v;
        } else if let Some(position) = star {
            p = position + 1;
            matched += 1;
            v = matched;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

/// Files whose content differs from the published build, including new and removed
/// files, in path order.
pub fn pending_paths(
    published: &BTreeMap<String, u64>,
    current: &BTreeMap<String, u64>,
) -> Vec<String> {
    let mut paths: Vec<String> = current
        .iter()
        .filter(|(path, hash)| published.get(*path) != Some(hash))
        .map(|(path, _)| path.clone())
        .chain(
            published
                .keys()
                .filter(|path| !current.contains_key(*path))
                .cloned(),
        )
        .collect();
    paths.sort();
    paths
}

pub struct Debouncer {
    delay_ms: u64,
    observed: Option<BTreeMap<String, u64>>,
    changed_at: Option<u64>,
}
impl Debouncer {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            delay_ms,
            observed: None,
            changed_at: None,
        }
    }
    /// A change was observed and is waiting for the debounce delay.
    pub fn pending(&self) -> bool {
        self.changed_at.is_some()
    }
    pub fn retry_after(&mut self, now_ms: u64) {
        self.changed_at = Some(now_ms);
    }
    pub fn observe(&mut self, files: &BTreeMap<String, u64>, now_ms: u64) -> bool {
        match &self.observed {
            None => {
                self.observed = Some(files.clone());
                return true;
            }
            Some(previous) if previous != files => {
                self.observed = Some(files.clone());
                self.changed_at = Some(now_ms);
            }
            _ => {}
        }
        if self
            .changed_at
            .is_some_and(|changed| now_ms.saturating_sub(changed) >= self.delay_ms)
        {
            self.changed_at = None;
            return true;
        }
        false
    }
}
