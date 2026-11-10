//! A bounded, process-local status read model, independent of graph transactions.
use serde::Serialize;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
pub struct ProgressSnapshot {
    pub run_id: u64,
    pub state: &'static str,
    pub stage: &'static str,
    pub detail: String,
    pub completed: usize,
    pub total: Option<usize>,
    pub unit: &'static str,
    #[serde(default)]
    pub files_completed: usize,
    #[serde(default)]
    pub files_total: Option<usize>,
    pub current_path: Option<String>,
    pub elapsed_ms: u64,
    pub stage_elapsed_ms: u64,
    pub eta_seconds: Option<u64>,
    pub error: Option<String>,
    pub poll_interval_ms: u64,
    pub workers: usize,
    pub active_workers: usize,
    /// A manual full reindex was requested and has not started yet.
    #[serde(default)]
    pub reindex_queued: bool,
    /// The watcher saw file changes that are not published yet (debounce running).
    #[serde(default)]
    pub changes_pending: bool,
    /// Up to 100 files changed since the last publication (also during a build).
    #[serde(default)]
    pub pending_paths: Vec<String>,
}

#[derive(Debug)]
struct ProgressState {
    snapshot: ProgressSnapshot,
    started: Instant,
    phase_started: Instant,
    stopped: Option<Instant>,
    logged_at: Instant,
    logged_bucket: usize,
    seen_paths: BTreeSet<String>,
}

impl Default for ProgressState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            snapshot: ProgressSnapshot {
                run_id: 0,
                state: "waiting",
                stage: "waiting",
                detail: String::new(),
                completed: 0,
                total: None,
                unit: "items",
                files_completed: 0,
                files_total: None,
                current_path: None,
                elapsed_ms: 0,
                stage_elapsed_ms: 0,
                eta_seconds: None,
                error: None,
                poll_interval_ms: 1000,
                workers: 1,
                active_workers: 0,
                reindex_queued: false,
                changes_pending: false,
                pending_paths: Vec::new(),
            },
            started: now,
            phase_started: now,
            stopped: None,
            logged_at: now,
            logged_bucket: 0,
            seen_paths: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct IndexingProgress(Arc<Mutex<ProgressState>>);

impl IndexingProgress {
    pub fn snapshot(&self) -> ProgressSnapshot {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.snapshot_at(Instant::now())
    }

    pub fn start(&self, poll_interval_ms: u64) {
        self.change(true, |state| {
            let run_id = state.snapshot.run_id + 1;
            let (reindex_queued, changes_pending) = (
                state.snapshot.reindex_queued,
                state.snapshot.changes_pending,
            );
            let pending_paths = std::mem::take(&mut state.snapshot.pending_paths);
            *state = ProgressState::default();
            state.snapshot.run_id = run_id;
            state.snapshot.pending_paths = pending_paths;
            state.snapshot.reindex_queued = reindex_queued;
            state.snapshot.changes_pending = changes_pending;
            state.snapshot.state = "running";
            state.snapshot.stage = "discovery";
            state.snapshot.poll_interval_ms = poll_interval_ms;
        });
    }

    pub fn phase(
        &self,
        stage: &'static str,
        detail: &str,
        total: Option<usize>,
        unit: &'static str,
    ) {
        self.change(true, |state| {
            if state.snapshot.state != "running" {
                return;
            }
            state.phase_started = Instant::now();
            state.snapshot.stage = stage;
            state.snapshot.detail = detail.into();
            state.snapshot.total = total;
            state.snapshot.unit = unit;
            state.snapshot.completed = 0;
            state.snapshot.files_completed = 0;
            state.snapshot.current_path = None;
            state.snapshot.workers = 1;
            state.snapshot.active_workers = 0;
            state.logged_bucket = 0;
            state.seen_paths.clear();
        });
    }

    /// Publish scope size so logs can show files=done/total beside item progress.
    pub fn set_files_total(&self, total: usize) {
        self.change(true, |state| {
            state.snapshot.files_total = Some(total);
            state.snapshot.files_completed = 0;
            state.seen_paths.clear();
        });
    }

    /// `completed` counts finished units; `path` identifies the current work.
    pub fn advance(&self, completed: usize, path: Option<&str>) {
        self.change(false, |state| {
            if state.snapshot.state != "running" {
                return;
            }
            state.snapshot.completed = completed.min(state.snapshot.total.unwrap_or(usize::MAX));
            if let Some(path) = path {
                if state.seen_paths.insert(path.to_owned()) {
                    let cap = state.snapshot.files_total.unwrap_or(usize::MAX);
                    state.snapshot.files_completed = state.seen_paths.len().min(cap);
                }
                state.snapshot.current_path = Some(path.to_owned());
            }
        });
    }

    pub fn set_workers(&self, workers: usize) {
        self.change(true, |state| {
            state.snapshot.workers = workers;
        });
    }

    pub fn work_started(&self, path: &str) {
        self.change(false, |state| {
            if state.snapshot.state != "running" {
                return;
            }
            state.snapshot.active_workers += 1;
            state.snapshot.current_path = Some(path.into());
        });
    }

    pub fn work_finished(&self, path: &str) {
        self.change(false, |state| {
            if state.snapshot.state != "running" {
                return;
            }
            state.snapshot.active_workers = state.snapshot.active_workers.saturating_sub(1);
            state.snapshot.completed += 1;
            state.snapshot.current_path = Some(path.into());
        });
    }

    /// Queue one full reindex; repeated requests before it starts coalesce.
    pub fn request_reindex(&self) {
        self.change(false, |state| state.snapshot.reindex_queued = true);
    }

    /// Files changed since the last publication; `changes_pending` follows from them.
    pub fn set_pending_paths(&self, mut paths: Vec<String>) {
        paths.truncate(100);
        let current = self.snapshot();
        if current.pending_paths != paths || current.changes_pending == paths.is_empty() {
            self.change(false, |state| {
                state.snapshot.changes_pending = !paths.is_empty();
                state.snapshot.pending_paths = paths;
            });
        }
    }

    pub fn set_changes_pending(&self, pending: bool) {
        if self.snapshot().changes_pending != pending {
            self.change(false, |state| state.snapshot.changes_pending = pending);
        }
    }

    /// Atomically consume a queued reindex request (watcher side).
    pub fn take_reindex_request(&self) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut state.snapshot.reindex_queued)
    }

    pub fn complete(&self) {
        self.stop("completed", None);
    }

    pub fn fail(&self, error: &str) {
        self.stop("failed", Some(error.to_owned()));
    }

    fn stop(&self, status: &'static str, error: Option<String>) {
        self.change(true, |state| {
            state.snapshot.state = status;
            state.snapshot.error = error;
            state.snapshot.active_workers = 0;
            state.stopped = Some(Instant::now());
        });
    }

    fn change(&self, force_log: bool, update: impl FnOnce(&mut ProgressState)) {
        let message = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            update(&mut state);
            state.log_message(force_log)
        };
        // stderr may be slow; never hold the status lock while writing it.
        if let Some(message) = message {
            eprintln!("{message}");
        }
    }
}

impl ProgressState {
    fn snapshot_at(&self, now: Instant) -> ProgressSnapshot {
        let mut snapshot = self.snapshot.clone();
        if snapshot.state == "waiting" {
            return snapshot;
        }
        let end = self.stopped.unwrap_or(now);
        let elapsed = end.saturating_duration_since(self.started);
        let phase_elapsed = end.saturating_duration_since(self.phase_started);
        snapshot.elapsed_ms = elapsed.as_millis() as u64;
        snapshot.stage_elapsed_ms = phase_elapsed.as_millis() as u64;
        snapshot.eta_seconds = if snapshot.state == "running" {
            estimate_remaining(snapshot.completed, snapshot.total, phase_elapsed)
        } else {
            None
        };
        snapshot
    }

    fn log_message(&mut self, force: bool) -> Option<String> {
        if self.snapshot.state == "waiting" {
            return None;
        }
        let now = Instant::now();
        let snapshot = self.snapshot_at(now);
        let percent = snapshot
            .total
            .filter(|&total| total > 0)
            .map(|total| ((snapshot.completed as f64 / total as f64 * 100.0) as usize).min(100));
        // Cap at 10: completed may briefly exceed total (e.g. a second flow pass).
        let bucket = percent.unwrap_or(0).min(100) / 10;
        let since_log = now.duration_since(self.logged_at);
        if !force
            && (since_log < Duration::from_secs(1)
                || (bucket <= self.logged_bucket && since_log < Duration::from_secs(5)))
        {
            return None;
        }
        self.logged_at = now;
        self.logged_bucket = bucket;
        let work = match (snapshot.state, percent, snapshot.total) {
            ("completed", _, _) => "[##########] 100% committed".to_owned(),
            ("failed", _, _) => "[failed]".to_owned(),
            (_, Some(percent), Some(total)) => format!(
                "[{}{}] {percent}% ({}/{total} {})",
                "#".repeat(bucket),
                "-".repeat(10usize.saturating_sub(bucket)),
                snapshot.completed,
                snapshot.unit
            ),
            _ => "[working]".to_owned(),
        };
        let files = snapshot
            .files_total
            .map(|total| format!(" files={}/{}", snapshot.files_completed, total))
            .unwrap_or_default();
        let eta = snapshot
            .eta_seconds
            .map(|seconds| format!("~{seconds}s"))
            .unwrap_or_else(|| "unknown".into());
        Some(format!("indexing: run={} state={} stage={}/{} {}{} workers={}/{} elapsed={}s stage_eta={} path={:?} error={:?}", snapshot.run_id, snapshot.state, snapshot.stage, snapshot.detail, work, files, snapshot.active_workers, snapshot.workers, snapshot.elapsed_ms / 1000, eta, snapshot.current_path, snapshot.error))
    }
}

fn estimate_remaining(completed: usize, total: Option<usize>, elapsed: Duration) -> Option<u64> {
    let total = total?;
    if completed < 3 || elapsed < Duration::from_secs(1) {
        return None;
    }
    Some(
        (elapsed.as_secs_f64() / completed as f64 * total.saturating_sub(completed) as f64).ceil()
            as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_completion_never_loses_counts_and_reports_busy_workers() {
        let progress = IndexingProgress::default();
        progress.start(1000);
        progress.phase("analysis", "php_parse", Some(100), "files");
        progress.set_workers(4);
        let gate = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let progress = &progress;
                let gate = &gate;
                scope.spawn(move || {
                    progress.work_started("fixture.php");
                    gate.wait();
                    assert_eq!(progress.snapshot().active_workers, 4);
                    gate.wait();
                    progress.work_finished("fixture.php");
                    for _ in 1..25 {
                        progress.work_started("fixture.php");
                        progress.work_finished("fixture.php");
                    }
                });
            }
        });
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.completed, 100);
        assert_eq!(snapshot.workers, 4);
        assert_eq!(snapshot.active_workers, 0);
        progress.phase("publishing", "transaction", None, "items");
        assert_eq!(progress.snapshot().workers, 1);
    }

    #[test]
    fn terminal_logs_distinguish_committed_success_from_a_failed_full_phase() {
        let mut state = ProgressState::default();
        state.snapshot.state = "failed";
        state.snapshot.completed = 10;
        state.snapshot.total = Some(10);
        let failed = state.log_message(true).unwrap();
        assert!(failed.contains("[failed]"));
        assert!(!failed.contains("100%"));
        state.snapshot.state = "completed";
        state.snapshot.total = None;
        assert!(state
            .log_message(true)
            .unwrap()
            .contains("[##########] 100% committed"));
    }

    #[test]
    fn progress_bar_survives_completed_past_total() {
        // Second php_flow pass used to push completed > total; percent/10 > 10 then
        // panicked in "-".repeat(10 - bucket) with capacity overflow.
        let mut state = ProgressState::default();
        state.snapshot.state = "running";
        state.snapshot.completed = 20_000;
        state.snapshot.total = Some(13_314);
        state.snapshot.unit = "functions";
        let message = state.log_message(true).unwrap();
        assert!(message.contains("[##########] 100%"), "{message}");
        assert!(message.contains("20000/13314"), "{message}");
    }

    #[test]
    fn slow_work_logs_advancement_even_before_the_first_ten_percent() {
        let mut state = ProgressState::default();
        state.snapshot.state = "running";
        state.snapshot.completed = 3;
        state.snapshot.total = Some(1000);
        state.logged_at = Instant::now() - Duration::from_secs(6);
        assert!(state.log_message(false).unwrap().contains("3/1000"));
        assert!(
            state.log_message(false).is_none(),
            "frequent updates stay throttled"
        );
    }

    #[test]
    fn publishing_logs_include_file_totals() {
        let progress = IndexingProgress::default();
        progress.start(1000);
        progress.set_files_total(10);
        progress.phase("publishing", "prepare_edges", Some(100), "items");
        progress.advance(50, Some("a.php"));
        progress.advance(60, Some("b.php"));
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.files_total, Some(10));
        assert_eq!(snapshot.files_completed, 2);
        let mut state = ProgressState {
            snapshot,
            logged_at: Instant::now() - Duration::from_secs(6),
            ..ProgressState::default()
        };
        state.snapshot.state = "running";
        let message = state.log_message(false).unwrap();
        assert!(message.contains("files=2/10"), "{message}");
    }

    #[test]
    fn eta_requires_measured_work_and_resets_for_each_phase() {
        assert_eq!(
            estimate_remaining(2, Some(10), Duration::from_secs(4)),
            None
        );
        assert_eq!(estimate_remaining(3, None, Duration::from_secs(4)), None);
        assert_eq!(
            estimate_remaining(3, Some(10), Duration::from_millis(500)),
            None
        );
        assert_eq!(
            estimate_remaining(3, Some(12), Duration::from_secs(6)),
            Some(18)
        );
        assert_eq!(
            estimate_remaining(12, Some(12), Duration::from_secs(6)),
            Some(0)
        );
        assert_eq!(estimate_remaining(0, Some(0), Duration::from_secs(6)), None);
        let progress = IndexingProgress::default();
        progress.start(1000);
        progress.phase("reading", "sources", Some(12), "files");
        progress.advance(3, Some("A.php"));
        progress.phase("analysis", "php", None, "items");
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.completed, 0);
        assert_eq!(snapshot.total, None);
        assert_eq!(snapshot.eta_seconds, None);
        assert_eq!(snapshot.current_path, None);
    }
}
