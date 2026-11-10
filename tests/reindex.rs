//! web-explore-and-reindex: manual reindex requests (INDEX-07).
use code_atlas::progress::IndexingProgress;

#[test]
fn reindex_request_is_idempotent_reported_and_consumed_once() {
    let progress = IndexingProgress::default();
    assert!(!progress.snapshot().reindex_queued);
    assert!(!progress.take_reindex_request());

    progress.request_reindex();
    progress.request_reindex();
    assert!(progress.snapshot().reindex_queued);

    assert!(progress.take_reindex_request());
    assert!(!progress.snapshot().reindex_queued);
    assert!(!progress.take_reindex_request(), "two presses run once");
}

#[test]
fn request_during_a_run_survives_the_run_start_and_completion() {
    let progress = IndexingProgress::default();
    progress.start(1000);
    progress.request_reindex();
    progress.phase("analysis", "php_flow", Some(10), "functions");
    progress.complete();
    assert!(progress.snapshot().reindex_queued);
    progress.start(1000);
    assert!(progress.snapshot().reindex_queued, "start does not consume");
    assert!(progress.take_reindex_request());
}
