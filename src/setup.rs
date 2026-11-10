//! `.atlas/setup.yaml`: project-owned analysis, search and MCP settings.
//!
//! The file is translated into the same `ATLAS_*` key map the environment used, so defaults,
//! ranges and validation stay in `Config::from_map` (Adapter). Secrets, resource limits and
//! deployment wiring stay in the environment.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Location of the setup file under the project root.
pub const SETUP_PATH: &str = ".atlas/setup.yaml";

/// Environment key ↔ YAML path of every setting that moved into the file.
pub const MOVED_KEYS: [(&str, &str); 18] = [
    ("ATLAS_WATCH_DIRS", "index.watch"),
    ("ATLAS_EXCLUDE", "exclude / index.exclude"),
    ("ATLAS_INCLUDE_VENDOR", "index.include_vendor"),
    ("ATLAS_DECLARATION_PATHS", "index.declaration_paths"),
    ("ATLAS_LANGUAGE_PLUGINS", "index.languages"),
    ("ATLAS_FRAMEWORK_PLUGINS", "index.frameworks"),
    ("ATLAS_PHP_VERSION", "index.php_version"),
    ("ATLAS_MAX_FILE_BYTES", "index.max_file_bytes"),
    ("ATLAS_MAX_ANALYSIS_BYTES", "index.max_analysis_bytes"),
    ("ATLAS_MAX_GRAPH_ELEMENTS", "index.max_graph_elements"),
    ("ATLAS_ANALYSIS_TIMEOUT_SECS", "index.analysis_timeout_secs"),
    ("ATLAS_DEBOUNCE_MS", "index.debounce_ms"),
    ("ATLAS_POLL_MS", "index.poll_ms"),
    ("ATLAS_GREP_MAX_FILE_BYTES", "grep.max_file_bytes"),
    ("ATLAS_TEST_PATHS", "tests.paths"),
    ("ATLAS_MAX_CONTEXT_NODES", "mcp.max_context_nodes"),
    ("ATLAS_MAX_CONTEXT_DEPTH", "mcp.max_context_depth"),
    ("ATLAS_MAX_REQUEST_BYTES", "mcp.max_request_bytes"),
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupFile {
    pub version: u32,
    /// Never indexed, searched or read.
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub index: IndexSection,
    #[serde(default)]
    pub grep: GrepSection,
    #[serde(default)]
    pub tests: TestsSection,
    #[serde(default)]
    pub mcp: McpSection,
    /// Prefix → folders of an application (label, Yii context, namespace-less names).
    #[serde(default)]
    pub areas: BTreeMap<String, AreaSection>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaSection {
    pub paths: Vec<String>,
    #[serde(default)]
    pub shared: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexSection {
    pub watch: Option<Vec<String>>,
    /// Graph only.
    #[serde(default)]
    pub exclude: Vec<String>,
    pub include_vendor: Option<bool>,
    pub declaration_paths: Option<Vec<String>>,
    pub languages: Option<Vec<String>>,
    pub frameworks: Option<Vec<String>>,
    pub php_version: Option<Version>,
    pub max_file_bytes: Option<u64>,
    pub max_analysis_bytes: Option<u64>,
    pub max_graph_elements: Option<u64>,
    pub analysis_timeout_secs: Option<u64>,
    pub debounce_ms: Option<u64>,
    pub poll_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrepSection {
    /// Text search only.
    #[serde(default)]
    pub exclude: Vec<String>,
    pub max_file_bytes: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestsSection {
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSection {
    pub max_context_nodes: Option<u64>,
    pub max_context_depth: Option<u64>,
    pub max_request_bytes: Option<u64>,
}

/// `php_version: 8.2` (a YAML number) or `"8.2"`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Version {
    Text(String),
    Number(f64),
}

impl Version {
    fn as_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            // 8.0 must stay "8.0", not "8".
            Self::Number(number) if number.fract() == 0.0 => format!("{number:.1}"),
            Self::Number(number) => number.to_string(),
        }
    }
}

/// The parsed file, `None` when the project has none.
pub fn read(project_root: &Path) -> Result<Option<SetupFile>, String> {
    let path = project_root.join(SETUP_PATH);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{SETUP_PATH}: {error}")),
    };
    let setup: SetupFile =
        serde_norway::from_str(&text).map_err(|error| format!("{SETUP_PATH}: {error}"))?;
    if setup.version != 1 {
        return Err(format!(
            "{SETUP_PATH}: version must be 1 (found {})",
            setup.version
        ));
    }
    Ok(Some(setup))
}

impl SetupFile {
    /// The file as `ATLAS_*` settings for the shared validation.
    pub fn settings(&self) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        let mut put = |key: &str, value: Option<String>| {
            if let Some(value) = value {
                values.insert(key.to_owned(), value);
            }
        };
        let list = |items: &Option<Vec<String>>| items.as_ref().map(|items| json_list(items));
        let number = |value: Option<u64>| value.map(|value| value.to_string());
        let index = &self.index;
        put("ATLAS_WATCH_DIRS", list(&index.watch));
        put("ATLAS_EXCLUDE", Some(json_list(&self.index_excludes())));
        put(
            "ATLAS_INCLUDE_VENDOR",
            index.include_vendor.map(|v| v.to_string()),
        );
        put("ATLAS_DECLARATION_PATHS", list(&index.declaration_paths));
        put("ATLAS_LANGUAGE_PLUGINS", list(&index.languages));
        put("ATLAS_FRAMEWORK_PLUGINS", list(&index.frameworks));
        put(
            "ATLAS_PHP_VERSION",
            index.php_version.as_ref().map(Version::as_text),
        );
        put("ATLAS_MAX_FILE_BYTES", number(index.max_file_bytes));
        put("ATLAS_MAX_ANALYSIS_BYTES", number(index.max_analysis_bytes));
        put("ATLAS_MAX_GRAPH_ELEMENTS", number(index.max_graph_elements));
        put(
            "ATLAS_ANALYSIS_TIMEOUT_SECS",
            number(index.analysis_timeout_secs),
        );
        put("ATLAS_DEBOUNCE_MS", number(index.debounce_ms));
        put("ATLAS_POLL_MS", number(index.poll_ms));
        put(
            "ATLAS_GREP_MAX_FILE_BYTES",
            number(self.grep.max_file_bytes),
        );
        put("ATLAS_TEST_PATHS", list(&self.tests.paths));
        put(
            "ATLAS_MAX_CONTEXT_NODES",
            number(self.mcp.max_context_nodes),
        );
        put(
            "ATLAS_MAX_CONTEXT_DEPTH",
            number(self.mcp.max_context_depth),
        );
        put(
            "ATLAS_MAX_REQUEST_BYTES",
            number(self.mcp.max_request_bytes),
        );
        values
    }

    /// Validated areas, ordered by prefix.
    pub fn areas(&self) -> Result<Vec<crate::domain::Area>, String> {
        let mut areas = Vec::new();
        for (prefix, section) in &self.areas {
            let valid_prefix = prefix
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
                && prefix
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !valid_prefix {
                return Err(format!(
                    "areas.{prefix} in {SETUP_PATH}: prefix must match [a-z][a-z0-9_]*"
                ));
            }
            if section.paths.is_empty() || section.paths.iter().any(|p| !relative_folder(p)) {
                return Err(format!(
                    "areas.{prefix}.paths in {SETUP_PATH} must list relative folders inside the project"
                ));
            }
            areas.push(crate::domain::Area {
                prefix: prefix.clone(),
                paths: section.paths.clone(),
                shared: section.shared,
            });
        }
        Ok(areas)
    }

    /// `exclude` + `index.exclude`.
    pub fn index_excludes(&self) -> Vec<String> {
        concat(&self.exclude, &self.index.exclude)
    }

    /// `exclude` + `grep.exclude`.
    pub fn grep_excludes(&self) -> Vec<String> {
        concat(&self.exclude, &self.grep.exclude)
    }
}

/// Moved keys present in the environment while the file exists.
pub fn conflicting_keys(environment: &BTreeMap<String, String>) -> Vec<String> {
    MOVED_KEYS
        .iter()
        .filter(|(key, _)| environment.contains_key(*key))
        .map(|(key, path)| format!("{key} (→ {path})"))
        .collect()
}

/// Validation messages name the YAML path instead of the internal key.
pub fn rename_keys(message: &str) -> String {
    MOVED_KEYS
        .iter()
        .fold(message.to_owned(), |text, (key, path)| {
            text.replace(key, &format!("{path} in {SETUP_PATH}"))
        })
}

fn relative_folder(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && !Path::new(value)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
}

fn concat(first: &[String], second: &[String]) -> Vec<String> {
    first.iter().chain(second).cloned().collect()
}

fn json_list(items: &[String]) -> String {
    serde_json::to_string(items).expect("strings serialize")
}
