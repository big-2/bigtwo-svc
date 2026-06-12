use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, RwLock};

use super::messages::WebSocketMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionToken(pub u64);

type PlayerConnection = (ConnectionToken, mpsc::Sender<String>);

/// Maps player to their outbound channel
/// Used by upstream components to send messages to players
/// The sender is a channel that directs into the Connection struct
/// The owned sender is called the outbound sender
#[async_trait]
pub trait ConnectionManager: Send + Sync {
    async fn add_connection(&self, uuid: String, sender: mpsc::Sender<String>) -> ConnectionToken;

    #[allow(dead_code)] // Direct removal remains useful for tests and administrative cleanup paths.
    async fn remove_connection(&self, uuid: &str);

    async fn remove_connection_if_current(&self, uuid: &str, token: ConnectionToken) -> bool;

    async fn send_to_player(&self, uuid: &str, message: &str);

    #[allow(dead_code)] // Trait method for batch messaging
    async fn send_to_players(&self, uuids: &[String], message: &str);

    async fn count_online_players(&self) -> usize;
}

pub struct InMemoryConnectionManager {
    // uuid -> sender plus connection generation token
    connections: Arc<RwLock<HashMap<String, PlayerConnection>>>,
    next_token: Arc<RwLock<u64>>,
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
            next_token: Arc::new(RwLock::new(0)),
        }
    }
}

#[async_trait]
impl ConnectionManager for InMemoryConnectionManager {
    async fn add_connection(&self, uuid: String, sender: mpsc::Sender<String>) -> ConnectionToken {
        let token = {
            let mut next_token = self.next_token.write().await;
            *next_token += 1;
            ConnectionToken(*next_token)
        };

        let mut connections = self.connections.write().await;

        // If there's an existing connection for this username, close it first
        if let Some((existing_token, existing_sender)) =
            connections.insert(uuid.clone(), (token, sender))
        {
            if let Ok(message) = serde_json::to_string(&WebSocketMessage::connected_elsewhere()) {
                let _ = existing_sender.try_send(message);
            }

            // Drop the existing sender to close the connection.
            drop(existing_sender);
            tracing::debug!(
                uuid = %uuid,
                old_token = existing_token.0,
                new_token = token.0,
                "Replaced existing WebSocket connection"
            );
        } else {
            tracing::debug!(uuid = %uuid, token = token.0, "Added new WebSocket connection");
        }

        token
    }

    async fn remove_connection(&self, uuid: &str) {
        let mut connections = self.connections.write().await;
        connections.remove(uuid);
    }

    async fn remove_connection_if_current(&self, uuid: &str, token: ConnectionToken) -> bool {
        let mut connections = self.connections.write().await;

        let should_remove = connections
            .get(uuid)
            .map(|(current_token, _)| *current_token == token)
            .unwrap_or(false);

        if should_remove {
            connections.remove(uuid);
            tracing::debug!(uuid = %uuid, token = token.0, "Removed current WebSocket connection");
            true
        } else {
            tracing::debug!(
                uuid = %uuid,
                token = token.0,
                "Skipped stale WebSocket cleanup because a newer connection exists"
            );
            false
        }
    }

    async fn send_to_player(&self, uuid: &str, message: &str) {
        let connection = {
            let connections = self.connections.read().await;
            connections
                .get(uuid)
                .map(|(token, sender)| (*token, sender.clone()))
        };

        if let Some((token, sender)) = connection {
            match sender.try_send(message.to_string()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    tracing::warn!(uuid = %uuid, "Dropped outbound WebSocket message due to full queue");
                }
                Err(TrySendError::Closed(_)) => {
                    tracing::debug!(uuid = %uuid, "Removing closed WebSocket connection");
                    self.remove_connection_if_current(uuid, token).await;
                }
            }
        }
    }

    async fn send_to_players(&self, uuids: &[String], message: &str) {
        let connections: Vec<(String, ConnectionToken, mpsc::Sender<String>)> = {
            let connections = self.connections.read().await;
            uuids
                .iter()
                .filter_map(|uuid| {
                    connections
                        .get(uuid)
                        .map(|(token, sender)| (uuid.clone(), *token, sender.clone()))
                })
                .collect()
        };

        let mut closed_connections = Vec::new();
        for uuid in uuids {
            if let Some((_, token, sender)) = connections
                .iter()
                .find(|(candidate, _, _)| candidate == uuid)
            {
                match sender.try_send(message.to_string()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        tracing::warn!(uuid = %uuid, "Dropped outbound WebSocket message due to full queue");
                    }
                    Err(TrySendError::Closed(_)) => {
                        closed_connections.push((uuid.clone(), *token));
                    }
                }
            }
        }

        for (uuid, token) in closed_connections {
            self.remove_connection_if_current(&uuid, token).await;
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
        let old_token = manager.add_connection("u1".to_string(), tx_old).await;

        let (tx_new, mut rx_new) = mpsc::channel::<String>(4);
        manager.add_connection("u1".to_string(), tx_new).await; // replace

        manager.send_to_player("u1", "only-new").await;

        // Old channel gets a terminal duplicate-tab message, then closes.
        let duplicate_notice = rx_old.recv().await.unwrap();
        assert!(duplicate_notice.contains("connected_elsewhere"));
        assert!(rx_old.recv().await.is_none());

        // New should receive
        let got = rx_new.recv().await.unwrap();
        assert_eq!(got, "only-new");

        // Stale cleanup from old connection must not remove newer connection.
        assert!(!manager.remove_connection_if_current("u1", old_token).await);
        manager.send_to_player("u1", "still-new").await;
        let got = rx_new.recv().await.unwrap();
        assert_eq!(got, "still-new");
    }
}
