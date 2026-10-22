//! Tests for the dedicated runtime used by blocking x402 callers.

use super::*;

#[test]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
fn bridge_debug_and_startup_errors_keep_their_context() {
    let bridge = Blocking::new().unwrap();
    assert_eq!(format!("{bridge:?}"), "Blocking { .. }");

    let source = std::io::Error::other("runtime cause");
    assert!(
        runtime_start_error(source)
            .to_string()
            .contains("cannot build")
    );
    let source = std::io::Error::other("thread cause");
    assert!(
        thread_start_error(source)
            .to_string()
            .contains("cannot start")
    );
}

#[test]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
fn bridge_reports_a_stopped_worker_and_reentrant_calls() {
    let (jobs, receiver) = mpsc::unbounded_channel();
    drop(receiver);
    let stopped = Blocking { jobs };
    let error = stopped.run(async { 1 }).unwrap_err();
    assert!(error.to_string().contains("bridge has stopped"));

    let bridge = Blocking::new().unwrap();
    let nested = bridge.clone();
    let result = bridge
        .run(async move { nested.run(async { 7 }).map_err(|error| error.to_string()) })
        .unwrap();
    assert!(
        result
            .unwrap_err()
            .contains("called back into its blocking bridge")
    );
}

#[test]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
fn bridge_runs_from_current_and_multithread_tokio_contexts() {
    let bridge = Blocking::new().unwrap();
    let current = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(
        current.block_on(async { bridge.run(async { 3 }).unwrap() }),
        3
    );

    let multi = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(
        multi.block_on(async { bridge.run(async { 5 }).unwrap() }),
        5
    );
}
