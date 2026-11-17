//! Append-only on-disk graph chunks so analysis can free RAM while more work runs.
//!
//! Layout matches the coordinator spill used by `index_spilled`:
//! `dir/nodes.jsonl` and `dir/edges.jsonl` (one JSON object per line).

use crate::domain::{Edge, Node};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct GraphSpill {
    dir: PathBuf,
    nodes: BufWriter<File>,
    edges: BufWriter<File>,
    pub node_count: usize,
    pub edge_count: usize,
}

impl GraphSpill {
    pub fn create() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("atlas-chunk-spill-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let nodes =
            BufWriter::new(File::create(dir.join("nodes.jsonl")).map_err(|e| e.to_string())?);
        let edges =
            BufWriter::new(File::create(dir.join("edges.jsonl")).map_err(|e| e.to_string())?);
        Ok(Self {
            dir,
            nodes,
            edges,
            node_count: 0,
            edge_count: 0,
        })
    }

    pub fn open(dir: PathBuf) -> Result<Self, String> {
        let nodes = BufWriter::new(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("nodes.jsonl"))
                .map_err(|e| e.to_string())?,
        );
        let edges = BufWriter::new(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("edges.jsonl"))
                .map_err(|e| e.to_string())?,
        );
        Ok(Self {
            dir,
            nodes,
            edges,
            node_count: 0,
            edge_count: 0,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn append_nodes(&mut self, nodes: impl IntoIterator<Item = Node>) -> Result<(), String> {
        for node in nodes {
            serde_json::to_writer(&mut self.nodes, &node).map_err(|e| e.to_string())?;
            self.nodes.write_all(b"\n").map_err(|e| e.to_string())?;
            self.node_count += 1;
        }
        Ok(())
    }

    pub fn append_edges(&mut self, edges: impl IntoIterator<Item = Edge>) -> Result<(), String> {
        for edge in edges {
            serde_json::to_writer(&mut self.edges, &edge).map_err(|e| e.to_string())?;
            self.edges.write_all(b"\n").map_err(|e| e.to_string())?;
            self.edge_count += 1;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.nodes.flush().map_err(|e| e.to_string())?;
        self.edges.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Flush writers and return the spill directory (files stay for `index_spilled`).
    pub fn finish(mut self) -> Result<PathBuf, String> {
        self.flush()?;
        Ok(self.dir)
    }
}

/// Append in-memory snapshot rows onto an existing spill directory.
pub fn append_snapshot(dir: &Path, nodes: &[Node], edges: &[Edge]) -> Result<(), String> {
    let mut spill = GraphSpill::open(dir.to_path_buf())?;
    spill.append_nodes(nodes.iter().cloned())?;
    spill.append_edges(edges.iter().cloned())?;
    spill.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SourceSpan;
    use serde_json::json;
    use std::io::{BufRead, BufReader};

    #[test]
    fn append_and_reread_roundtrip() {
        let mut spill = GraphSpill::create().unwrap();
        spill
            .append_nodes([Node {
                id: "function:a".into(),
                kind: "function".into(),
                name: "a".into(),
                qualified_name: "a".into(),
                span: SourceSpan {
                    path: "a.php".into(),
                    start_line: 1,
                    end_line: 1,
                },
                metadata: json!({}),
                trust: 1,
                revision: 0,
            }])
            .unwrap();
        spill
            .append_edges([Edge {
                id: "e1".into(),
                kind: "contains".into(),
                from: "function:a".into(),
                to: None,
                span: SourceSpan {
                    path: "a.php".into(),
                    start_line: 1,
                    end_line: 1,
                },
                metadata: json!({}),
                trust: 1,
                revision: 0,
            }])
            .unwrap();
        let dir = spill.finish().unwrap();
        let lines: Vec<_> = BufReader::new(File::open(dir.join("nodes.jsonl")).unwrap())
            .lines()
            .map(|l| l.unwrap())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("function:a"));
        let _ = fs::remove_dir_all(dir);
    }
}
