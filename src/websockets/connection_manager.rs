use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, RwLock};

/// Maps player to their outbound channel
/// Used by upstream components to send messages to players
/// The sender is a channel that directs into the Connection struct
/// The owned sender is called the outbound sender
#[async_trait]
pub trait ConnectionManager: Send + Sync {
    async fn add_connection(&self, uuid: String, sender: mpsc::Sender<String>);

    async fn remove_connection(&self, uuid: &str);

    async fn send_to_player(&self, uuid: &str, message: &str);

    #[allow(dead_code)] // Trait method for batch messaging
    async fn send_to_players(&self, uuids: &[String], message: &str);

    async fn count_online_players(&self) -> usize;
}

pub struct InMemoryConnectionManager {
    // uuid -> sender
    connections: Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>,
}

impl Default for InMemoryConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl ConnectionManager for InMemoryConnectionManager {
    async fn add_connection(&self, uuid: String, sender: mpsc::Sender<String>) {
        let mut connections = self.connections.write().await;

        // If there's an existing connection for this username, close it first
        if let Some(existing_sender) = connections.insert(uuid.clone(), sender) {
            // Drop the existing sender to close the connection
            drop(existing_sender);
            tracing::debug!(uuid = %uuid, "Replaced existing WebSocket connection");
        } else {
            tracing::debug!(uuid = %uuid, "Added new WebSocket connection");
        }
    }

    async fn remove_connection(&self, uuid: &str) {
        let mut connections = self.connections.write().await;
        connections.remove(uuid);
    }

    async fn send_to_player(&self, uuid: &str, message: &str) {
        let sender = {
            let connections = self.connections.read().await;
            connections.get(uuid).cloned()
        };

        if let Some(sender) = sender {
            match sender.try_send(message.to_string()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    tracing::warn!(uuid = %uuid, "Dropped outbound WebSocket message due to full queue");
                }
                Err(TrySendError::Closed(_)) => {
                    tracing::debug!(uuid = %uuid, "Removing closed WebSocket connection");
                    self.remove_connection(uuid).await;
                }
            }
        }
    }

    async fn send_to_players(&self, uuids: &[String], message: &str) {
        let connections: Vec<(String, mpsc::Sender<String>)> = {
            let connections = self.connections.read().await;
            uuids
                .iter()
                .filter_map(|uuid| {
                    connections
                        .get(uuid)
                        .cloned()
                        .map(|sender| (uuid.clone(), sender))
                })
                .collect()
        };

        let mut closed_connections = Vec::new();
        for uuid in uuids {
            if let Some((_, sender)) = connections.iter().find(|(candidate, _)| candidate == uuid) {
                match sender.try_send(message.to_string()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        tracing::warn!(uuid = %uuid, "Dropped outbound WebSocket message due to full queue");
                    }
                    Err(TrySendError::Closed(_)) => {
                        closed_connections.push(uuid.clone());
                    }
                }
            }
        }

        for uuid in closed_connections {
            self.remove_connection(&uuid).await;
        }
    }

    async fn count_online_players(&self) -> usize {
        let connections = self.connections.read().await;
        connections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_and_send_to_single_player() {
        let manager = InMemoryConnectionManager::new();

        let (tx, mut rx) = mpsc::channel::<String>(4);
        manager.add_connection("u1".to_string(), tx).await;

        manager.send_to_player("u1", "hello").await;
        let got = rx.recv().await.unwrap();
        assert_eq!(got, "hello");
    }

    #[tokio::test]
    async fn test_send_to_multiple_players() {
        let manager = InMemoryConnectionManager::new();

        let (tx1, mut rx1) = mpsc::channel::<String>(4);
        let (tx2, mut rx2) = mpsc::channel::<String>(4);
        manager.add_connection("u1".to_string(), tx1).await;
        manager.add_connection("u2".to_string(), tx2).await;

        manager
            .send_to_players(&["u1".to_string(), "u2".to_string()], "msg")
            .await;

        let a = rx1.recv().await.unwrap();
        let b = rx2.recv().await.unwrap();
        assert_eq!(a, "msg");
        assert_eq!(b, "msg");
    }

    #[tokio::test]
    async fn test_remove_connection() {
        let manager = InMemoryConnectionManager::new();

        let (tx, mut rx) = mpsc::channel::<String>(4);
        manager.add_connection("u1".to_string(), tx).await;

        manager.remove_connection("u1").await;
        manager.send_to_player("u1", "nope").await;

        // Channel should be closed; recv returns None
        let res = rx.recv().await;
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn test_replace_existing_connection_uses_new_sender() {
        let manager = InMemoryConnectionManager::new();

        let (tx_old, mut rx_old) = mpsc::channel::<String>(4);
        manager.add_connection("u1".to_string(), tx_old).await;

        let (tx_new, mut rx_new) = mpsc::channel::<String>(4);
        manager.add_connection("u1".to_string(), tx_new).await; // replace

        manager.send_to_player("u1", "only-new").await;

        // Old channel should be closed
        let res_old = rx_old.recv().await;
        assert!(res_old.is_none());

        // New should receive
        let got = rx_new.recv().await.unwrap();
        assert_eq!(got, "only-new");
    }
}
