use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

/// One lock per job name, so a job's runs never overlap: scheduled and triggered runs of the
/// same job wait their turn, in the order they arrived.
#[derive(Clone, Default)]
pub struct JobLocks(Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>);

impl JobLocks {
    /// The lock for job `name`, created on first use.
    pub async fn get(&self, name: &str) -> Arc<Mutex<()>> {
        self.0
            .lock()
            .await
            .entry(name.to_string())
            .or_default()
            .clone()
    }
}
