//! Hot-adjustable worker pool targets (no process restart).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct WorkerPoolConfig {
    pub analyze: Arc<AtomicUsize>,
    pub ingest: Arc<AtomicUsize>,
    pub gateway: Arc<AtomicUsize>,
}

impl WorkerPoolConfig {
    pub fn new(analyze: usize, ingest: usize, gateway: usize) -> Self {
        Self {
            analyze: Arc::new(AtomicUsize::new(analyze.max(1))),
            ingest: Arc::new(AtomicUsize::new(ingest.max(1))),
            gateway: Arc::new(AtomicUsize::new(gateway.max(1))),
        }
    }

    pub fn analyze(&self) -> usize {
        self.analyze.load(Ordering::Relaxed).max(1)
    }

    pub fn set_analyze(&self, n: usize) {
        self.analyze.store(n.max(1), Ordering::Relaxed);
    }
}
