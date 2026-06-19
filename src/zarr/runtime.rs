use std::sync::OnceLock;

use zarrs::storage::storage_adapter::async_to_sync::AsyncToSyncBlockOn;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Shared tokio runtime used to bridge async object-store I/O into the sync zarrs API.
pub fn shared_runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for S3 I/O")
    })
}

#[derive(Clone)]
pub struct TokioBlockOn(tokio::runtime::Handle);

impl TokioBlockOn {
    pub fn shared() -> Self {
        Self(shared_runtime().handle().clone())
    }
}

impl AsyncToSyncBlockOn for TokioBlockOn {
    fn block_on<F: core::future::Future>(&self, future: F) -> F::Output {
        self.0.block_on(future)
    }
}
