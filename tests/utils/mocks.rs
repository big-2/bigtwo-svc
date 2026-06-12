//! Mock implementations for testing
#![allow(dead_code)] // Test utilities may not all be used in every test

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use bigtwo::websockets::{ConnectionManager, ConnectionToken};

// ============================================================================
// Mock Infrastructure
// ============================================================================

#[derive(Clone)]
pub struct MockConnectionManager {
    sent_messages: Arc<RwLock<HashMap<String, Vec<String>>>>,
    connected_players: Arc<RwLock<Vec<String>>>,
    name_to_uuid: Arc<RwLock<HashMap<String, String>>>,
}

impl Default for MockConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MockConnectionManager {
    pub fn new() -> Self {
        Self {
            sent_messages: Arc::new(RwLock::new(HashMap::new())),
            connected_players: Arc::new(RwLock::new(Vec::new())),
            name_to_uuid: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_connected_player(&self, username: &str) {
        self.connected_players
            .write()
            .await
            .push(username.to_string());
    }

    pub async fn get_messages_for(&self, username: &str) -> Vec<String> {
        self.sent_messages
            .read()
            .await
            .get(username)
            .cloned()
            .unwrap_or_default()
    }

    /// Consume and return the first message for a player (like a proper queue)
    pub async fn consume_message_for(&self, uuid: &str) -> Option<String> {
        let mut messages = self.sent_messages.write().await;
        let player_messages = messages.get_mut(uuid)?;
        if player_messages.is_empty() {
            None
        } else {
            Some(player_messages.remove(0))
        }
    }

    pub async fn clear_messages(&self) {
        self.sent_messages.write().await.clear();
    }

    /// Register a mapping from player name to uuid (for tests)
    pub async fn register_player_mapping(&self, name: &str, uuid: &str) {
        self.name_to_uuid
            .write()
            .await
            .insert(name.to_string(), uuid.to_string());
    }
}

#[async_trait]
impl ConnectionManager for MockConnectionManager {
    async fn add_connection(&self, uuid: String, _sender: mpsc::Sender<String>) -> ConnectionToken {
        self.add_connected_player(&uuid).await;
        ConnectionToken(0)
    }

    async fn remove_connection(&self, uuid: &str) {
        self.connected_players.write().await.retain(|p| p != uuid);
    }

    async fn remove_connection_if_current(&self, uuid: &str, _token: ConnectionToken) -> bool {
        self.remove_connection(uuid).await;
        true
    }

    async fn send_to_player(&self, uuid: &str, message: &str) {
        // In production the key is uuid. Some code paths in tests send to
        // player name (game_player.name). Normalize here by remapping names to uuids.
        let key = if let Some(mapped) = self.name_to_uuid.read().await.get(uuid).cloned() {
            mapped
        } else {
            uuid.to_string()
        };

        self.sent_messages
            .write()
            .await
            .entry(key)
            .or_default()
            .push(message.to_string());
    }

    async fn send_to_players(&self, uuids: &[String], message: &str) {
        for uuid in uuids {
            self.send_to_player(uuid, message).await;
        }
    }

    async fn count_online_players(&self) -> usize {
        self.connected_players.read().await.len()
    }
}
