//! Dedicated tokio bridge for the ledger's synchronous API.

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc as std_mpsc;
use std::thread;

use tinystoragedrivers_types::{Result, StorageError};
use tokio::sync::mpsc;

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

thread_local! {
    static ON_BRIDGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs storage-port futures on a dedicated runtime thread.
#[derive(Clone)]
pub(super) struct Blocking {
    jobs: mpsc::UnboundedSender<Job>,
}

impl std::fmt::Debug for Blocking {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Blocking").finish_non_exhaustive()
    }
}

impl Blocking {
    /// Start the dedicated runtime thread.
    pub(super) fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(runtime_start_error)?;
        let (jobs, mut receiver) = mpsc::unbounded_channel::<Job>();
        thread::Builder::new()
            .name("tinywallet-storage".into())
            .spawn(move || {
                ON_BRIDGE.with(|flag| flag.set(true));
                runtime.block_on(async move {
                    let mut running = tokio::task::JoinSet::new();
                    while let Some(job) = receiver.recv().await {
                        running.spawn(job);
                        while running.try_join_next().is_some() {}
                    }
                    while running.join_next().await.is_some() {}
                });
            })
            .map_err(thread_start_error)?;
        Ok(Self { jobs })
    }

    /// Run one future to completion and return its output.
    pub(super) fn run<F, T>(&self, future: F) -> Result<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        if ON_BRIDGE.with(std::cell::Cell::get) {
            return Err(StorageError::backend(
                "a ledger storage future called back into its blocking bridge",
            ));
        }
        let (reply, wait) = std_mpsc::sync_channel(1);
        let job: Job = Box::pin(async move {
            let _ = reply.send(future.await);
        });
        self.jobs
            .send(job)
            .map_err(|_| StorageError::backend("the ledger storage bridge has stopped"))?;
        let receive = || {
            wait.recv()
                .map_err(|_| StorageError::backend("the ledger storage bridge dropped a call"))
        };
        if tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        }) {
            tokio::task::block_in_place(receive)
        } else {
            receive()
        }
    }
}

fn runtime_start_error(error: std::io::Error) -> StorageError {
    StorageError::backend("cannot build the ledger storage runtime").with_source(error)
}

fn thread_start_error(error: std::io::Error) -> StorageError {
    StorageError::backend("cannot start the ledger storage thread").with_source(error)
}

#[cfg(test)]
#[path = "blocking_tests.rs"]
mod tests;
