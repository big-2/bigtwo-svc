use std::sync::Arc;
use std::time::Duration;

use tokio::time::interval;
use tracing::{error, info, instrument};

use super::service::SessionService;

#[allow(dead_code)] // Used by the binary crate to configure the background task
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    pub cleanup_interval: Duration,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            cleanup_interval: Duration::from_secs(30 * 60),
        }
    }
}

#[allow(dead_code)] // Invoked from the binary crate
#[instrument(skip(session_service))]
pub async fn start_cleanup_task(session_service: Arc<SessionService>, config: CleanupConfig) {
    info!(
        cleanup_interval_secs = config.cleanup_interval.as_secs(),
        "Starting session cleanup background task"
    );

    let mut cleanup_interval = interval(config.cleanup_interval);

    loop {
        cleanup_interval.tick().await;

        match session_service.cleanup_expired_sessions().await {
            Ok(removed_count) => {
                info!(
                    removed_sessions = removed_count,
                    "Session cleanup completed"
                );
            }
            Err(err) => {
                error!(error = %err, "Session cleanup task failed");
            }
        }
    }
}
