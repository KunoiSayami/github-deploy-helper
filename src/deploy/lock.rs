use dashmap::{DashMap, DashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct DeployLock {
    pub mutex: Mutex<()>,
    pub seen: DashSet<String>,
}

pub type DeployLockMap = Arc<DashMap<String, Arc<DeployLock>>>;
