//! Optional MCP response post-processor from `.atlas/setup.yaml` `mcp.response_script`.
//!
//! The script receives the tool text on stdin and must print the replacement on stdout.
//! Env: `ATLAS_MCP_TOOL`, `ATLAS_PROJECT_ROOT`. Relative path only; timeout-bounded.
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const DEFAULT_TIMEOUT_SECS: u64 = 5;
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpHook {
    /// Absolute path to a bash script under the project root.
    pub script: Option<PathBuf>,
    pub project_root: PathBuf,
    pub timeout_secs: u64,
}

impl McpHook {
    pub fn from_setup(
        project_root: &Path,
        relative: Option<&str>,
        timeout_secs: Option<u64>,
    ) -> Result<Self, String> {
        let timeout_secs = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).clamp(1, 30);
        let Some(relative) = relative.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(Self {
                script: None,
                project_root: project_root.to_path_buf(),
                timeout_secs,
            });
        };
        if Path::new(relative).is_absolute()
            || Path::new(relative)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(
                "mcp.response_script in .atlas/setup.yaml must be a relative path inside the project"
                    .into(),
            );
        }
        let path = project_root.join(relative);
        if !path.is_file() {
            return Err(format!(
                "mcp.response_script in .atlas/setup.yaml not found: {relative}"
            ));
        }
        Ok(Self {
            script: Some(path),
            project_root: project_root.to_path_buf(),
            timeout_secs,
        })
    }

    pub fn enabled(&self) -> bool {
        self.script.is_some()
    }
}

/// Runs the optional script; on failure keeps the original text and logs to stderr.
pub fn process_tool_text(hook: &McpHook, tool: &str, text: String) -> String {
    let Some(script) = &hook.script else {
        return text;
    };
    match run_script(script, &hook.project_root, tool, &text, hook.timeout_secs) {
        Ok(out) => out,
        Err(error) => {
            eprintln!("mcp.response_script ({tool}): {error}; using original response");
            text
        }
    }
}

fn run_script(
    script: &Path,
    project_root: &Path,
    tool: &str,
    text: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let mut child = Command::new("/bin/bash")
        .arg(script)
        .current_dir(project_root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/tmp")
        .env("ATLAS_MCP_TOOL", tool)
        .env("ATLAS_PROJECT_ROOT", project_root.display().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("write stdin: {e}"))?;
    }

    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {timeout_secs}s"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("wait: {e}")),
        }
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut stdout).map_err(|e| e.to_string())?;
    }
    if let Some(mut err) = child.stderr.take() {
        err.read_to_end(&mut stderr).map_err(|e| e.to_string())?;
    }
    if !status.success() {
        let err = String::from_utf8_lossy(&stderr);
        return Err(format!(
            "exit {}: {}",
            status.code().unwrap_or(-1),
            err.trim()
        ));
    }
    if stdout.len() > MAX_OUTPUT_BYTES {
        return Err(format!("stdout exceeds {MAX_OUTPUT_BYTES} bytes"));
    }
    String::from_utf8(stdout).map_err(|e| format!("stdout utf-8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_script(body: &str) -> (PathBuf, PathBuf) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "atlas-hook-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("hook.sh");
        std::fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (root, script)
    }

    #[test]
    fn script_transforms_stdin() {
        let (root, script) = temp_script("#!/bin/bash\ntr a-z A-Z\n");
        let hook = McpHook {
            script: Some(script),
            project_root: root,
            timeout_secs: 5,
        };
        let out = process_tool_text(&hook, "atlas_search", "hello".into());
        assert_eq!(out, "HELLO");
    }

    #[test]
    fn timeout_keeps_original() {
        let (root, script) = temp_script("#!/bin/bash\nsleep 3\ncat\n");
        let hook = McpHook {
            script: Some(script),
            project_root: root,
            timeout_secs: 1,
        };
        let out = process_tool_text(&hook, "atlas_search", "keep".into());
        assert_eq!(out, "keep");
    }

    #[test]
    fn relative_path_must_exist() {
        let root = std::env::temp_dir();
        let err = McpHook::from_setup(&root, Some("missing-hook.sh"), None).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }
}
