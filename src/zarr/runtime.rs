//! Shared Tokio runtime for bridging async S3 object-store I/O into sync zarrs APIs.

use std::sync::OnceLock;

use zarrs::storage::storage_adapter::async_to_sync::AsyncToSyncBlockOn;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Shared multi-thread Tokio runtime used for S3 reads via the zarrs async-to-sync adapter.
pub fn shared_runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for S3 I/O")
    })
}

/// [`AsyncToSyncBlockOn`] implementation that delegates to [`shared_runtime`].
#[derive(Clone)]
pub struct TokioBlockOn(tokio::runtime::Handle);

impl TokioBlockOn {
    /// Block on async work using the shared runtime handle.
    pub fn shared() -> Self {
        Self(shared_runtime().handle().clone())
    }
}

impl AsyncToSyncBlockOn for TokioBlockOn {
    fn block_on<F: core::future::Future>(&self, future: F) -> F::Output {
        self.0.block_on(future)
    }
}
