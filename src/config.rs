use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
};

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub project_root: PathBuf,
    pub bind: SocketAddr,
    pub debounce_ms: u64,
    pub poll_ms: u64,
    pub watch_dirs: Vec<String>,
    /// Index walk exclusions (`exclude` + `index.exclude`).
    pub excludes: Vec<String>,
    /// Text search walk exclusions (`exclude` + `grep.exclude`).
    pub grep_excludes: Vec<String>,
    /// Paths `atlas_read` refuses besides the built-in ones (`exclude`).
    pub read_excludes: Vec<String>,
    /// Application folders from `.atlas/setup.yaml` (empty without the file).
    pub areas: Vec<crate::domain::Area>,
    pub include_vendor: bool,
    /// Library paths indexed for declarations only, even when vendor is excluded.
    pub declaration_paths: Vec<String>,
    pub language_plugins: Vec<String>,
    pub framework_plugins: Vec<String>,
    pub language_versions: BTreeMap<String, String>,
    pub max_file_bytes: u64,
    /// Text search (`atlas_grep`, withGrep) reads files up to this size.
    pub grep_max_file_bytes: u64,
    /// Globs of test code left out of agent views unless `include_tests`.
    pub test_paths: Vec<String>,
    pub max_analysis_bytes: u64,
    pub max_graph_elements: usize,
    pub analysis_timeout_secs: u64,
    pub cpu_limit: f64,
    pub index_workers: usize,
    pub memory_limit_mb: u64,
    pub max_context_nodes: usize,
    pub max_context_depth: usize,
    pub max_request_bytes: usize,
    pub agent_token: String,
    pub review_token: String,
    pub allowed_origins: Vec<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("project_root", &self.project_root)
            .field("bind", &self.bind)
            .field("watch_dirs", &self.watch_dirs)
            .field("debounce_ms", &self.debounce_ms)
            .finish_non_exhaustive()
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_map(&std::env::vars().collect())
    }

    /// Environment plus `<ATLAS_PROJECT_ROOT>/.atlas/setup.yaml` when the project has one.
    pub fn from_map(values: &BTreeMap<String, String>) -> Result<Self, String> {
        let project_root = PathBuf::from(Environment(values).required("ATLAS_PROJECT_ROOT")?)
            .canonicalize()
            .map_err(|_| "ATLAS_PROJECT_ROOT must be an existing directory".to_owned())?;
        if !project_root.is_dir() {
            return Err("ATLAS_PROJECT_ROOT must be a directory".into());
        }
        let Some(setup) = crate::setup::read(&project_root)? else {
            return Self::from_settings(values, project_root);
        };
        let conflicts = crate::setup::conflicting_keys(values);
        if !conflicts.is_empty() {
            return Err(format!(
                "{} exists: remove these environment variables (now set in the file): {}",
                crate::setup::SETUP_PATH,
                conflicts.join(", ")
            ));
        }
        let mut merged = values.clone();
        merged.extend(setup.settings());
        let mut config = Self::from_settings(&merged, project_root)
            .map_err(|error| crate::setup::rename_keys(&error))?;
        config.grep_excludes = setup.grep_excludes();
        config.read_excludes = setup.exclude.clone();
        config.areas = setup.areas()?;
        Ok(config)
    }

    /// Validated settings from one key map (environment or translated setup file).
    fn from_settings(
        values: &BTreeMap<String, String>,
        project_root: PathBuf,
    ) -> Result<Self, String> {
        let env = Environment(values);
        let watch_dirs = env.list("ATLAS_WATCH_DIRS", "[\".\"]")?;
        if watch_dirs.is_empty() || watch_dirs.iter().any(|path| !relative_path(path)) {
            return Err(
                "ATLAS_WATCH_DIRS must contain relative directories within the project".into(),
            );
        }
        let agent_token = env.required("ATLAS_AGENT_TOKEN")?;
        let review_token = env.required("ATLAS_REVIEW_TOKEN")?;
        if agent_token == review_token {
            return Err("ATLAS_REVIEW_TOKEN must differ from ATLAS_AGENT_TOKEN".into());
        }
        let php_version = env.get("ATLAS_PHP_VERSION", "8.2");
        if ![
            "5.6", "7.0", "7.1", "7.2", "7.3", "7.4", "8.0", "8.1", "8.2", "8.3", "8.4", "8.5",
        ]
        .contains(&php_version)
        {
            return Err(
                "ATLAS_PHP_VERSION must be a supported major.minor version (5.6, 7.0–7.4, 8.0–8.5)"
                    .into(),
            );
        }
        for key in [
            "ATLAS_PHP_BINARY",
            "ATLAS_PHP_PLUGIN",
            "ATLAS_PHP_LINT",
            "ATLAS_YII",
            "ATLAS_MAX_PLUGIN_OUTPUT_BYTES",
            "ATLAS_PLUGIN_TIMEOUT_SECS",
        ] {
            if values.contains_key(key) {
                return Err(format!("{key} was removed: use ATLAS_LANGUAGE_PLUGINS, ATLAS_FRAMEWORK_PLUGINS and ATLAS_ANALYSIS_TIMEOUT_SECS; analysis is native"));
            }
        }
        let language_plugins = env.list("ATLAS_LANGUAGE_PLUGINS", "[\"php\"]")?;
        let framework_plugins = env.list("ATLAS_FRAMEWORK_PLUGINS", "[\"yii2\"]")?;
        crate::plugins::registry()
            .validate(&language_plugins, &framework_plugins)
            .map_err(|error| {
                format!("ATLAS_LANGUAGE_PLUGINS / ATLAS_FRAMEWORK_PLUGINS: {error}")
            })?;
        let bind: SocketAddr = env
            .get("ATLAS_BIND", "127.0.0.1:8080")
            .parse()
            .map_err(|_| "ATLAS_BIND must be an IP address and port".to_owned())?;
        let cpu_limit: f64 = env
            .get("ATLAS_CPU_LIMIT", "1")
            .parse()
            .map_err(|_| "ATLAS_CPU_LIMIT must be a positive number".to_owned())?;
        if !cpu_limit.is_finite() || cpu_limit <= 0.0 || cpu_limit > 256.0 {
            return Err("ATLAS_CPU_LIMIT must be finite and in (0,256]".into());
        }
        let requested_workers = env.number("ATLAS_INDEX_WORKERS", 0, 0, 32)? as usize;
        let index_workers = if requested_workers == 0 {
            cpu_limit.ceil().clamp(1.0, 32.0) as usize
        } else {
            requested_workers
        };
        let declaration_paths = env
            .list("ATLAS_DECLARATION_PATHS", "[\"vendor/yiisoft\"]")
            .map_err(|_| "ATLAS_DECLARATION_PATHS must be a JSON array of strings".to_owned())?;
        if declaration_paths.iter().any(|path| !relative_path(path)) {
            return Err(
                "ATLAS_DECLARATION_PATHS entries must be relative paths inside the project".into(),
            );
        }
        let origins_default = serde_json::to_string(&[
            format!("http://localhost:{}", bind.port()),
            format!("http://127.0.0.1:{}", bind.port()),
        ])
        .expect("strings serialize");
        let origins = env.list("ATLAS_ALLOWED_ORIGINS", &origins_default)?;
        if origins.iter().any(|origin| {
            origin == "*" || !(origin.starts_with("http://") || origin.starts_with("https://"))
        }) {
            return Err("ATLAS_ALLOWED_ORIGINS requires explicit HTTP(S) origins".into());
        }
        let excludes = env.list("ATLAS_EXCLUDE", "[]")?;
        Ok(Self {
            database_url: env.required("ATLAS_DATABASE_URL")?,
            project_root,
            bind,
            watch_dirs,
            grep_excludes: excludes.clone(),
            read_excludes: Vec::new(),
            areas: Vec::new(),
            excludes,
            declaration_paths,
            debounce_ms: env.number("ATLAS_DEBOUNCE_MS", 5000, 0, 3_600_000)?,
            poll_ms: env.number("ATLAS_POLL_MS", 1000, 1, 3_600_000)?,
            include_vendor: env.boolean("ATLAS_INCLUDE_VENDOR", false)?,
            language_plugins,
            framework_plugins,
            language_versions: BTreeMap::from([("php".into(), php_version.into())]),
            max_file_bytes: env.number("ATLAS_MAX_FILE_BYTES", 2_097_152, 1, 1_073_741_824)?,
            grep_max_file_bytes: env.number(
                "ATLAS_GREP_MAX_FILE_BYTES",
                16_777_216,
                1024,
                1_073_741_824,
            )?,
            test_paths: env.list(
                "ATLAS_TEST_PATHS",
                r#"["tests/*","*/tests/*","*Test.php","*Cest.php"]"#,
            )?,
            max_analysis_bytes: env.number(
                "ATLAS_MAX_ANALYSIS_BYTES",
                67_108_864,
                1024,
                1_073_741_824,
            )?,
            // Finer occurrence/branch nodes need headroom on mid-size PHP trees.
            max_graph_elements: env.number("ATLAS_MAX_GRAPH_ELEMENTS", 2_000_000, 1, 20_000_000)?
                as usize,
            analysis_timeout_secs: env.number("ATLAS_ANALYSIS_TIMEOUT_SECS", 120, 1, 3600)?,
            cpu_limit,
            index_workers,
            memory_limit_mb: env.number("ATLAS_MEMORY_LIMIT_MB", 1024, 64, 1_048_576)?,
            max_context_nodes: env.number("ATLAS_MAX_CONTEXT_NODES", 100, 1, 10_000)? as usize,
            max_context_depth: env.number("ATLAS_MAX_CONTEXT_DEPTH", 3, 0, 20)? as usize,
            max_request_bytes: env.number("ATLAS_MAX_REQUEST_BYTES", 1_048_576, 1024, 16_777_216)?
                as usize,
            agent_token,
            review_token,
            allowed_origins: origins,
        })
    }
}

fn relative_path(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && !Path::new(value).components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

struct Environment<'a>(&'a BTreeMap<String, String>);
impl Environment<'_> {
    fn get<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.0.get(key).map(String::as_str).unwrap_or(default)
    }
    fn required(&self, key: &str) -> Result<String, String> {
        self.0
            .get(key)
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or_else(|| format!("{key} is required"))
    }
    fn number(&self, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
        let value = match self.0.get(key) {
            None => default,
            Some(value) => value
                .parse()
                .map_err(|_| format!("{key} must be an integer"))?,
        };
        if !(min..=max).contains(&value) {
            return Err(format!("{key} must be in [{min},{max}]"));
        }
        Ok(value)
    }
    fn boolean(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.0.get(key).map(String::as_str) {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            _ => Err(format!("{key} must be true or false")),
        }
    }
    fn list(&self, key: &str, default: &str) -> Result<Vec<String>, String> {
        serde_json::from_str::<Vec<String>>(self.get(key, default))
            .map_err(|_| format!("{key} must be a JSON array of strings"))
    }
}
