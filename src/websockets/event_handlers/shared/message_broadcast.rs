use crate::{
    event::RoomEventError,
    websockets::{connection_manager::ConnectionManager, messages::WebSocketMessage},
};
use std::sync::Arc;

pub struct MessageBroadcaster;

impl MessageBroadcaster {
    pub async fn broadcast_to_players(
        connection_manager: &Arc<dyn ConnectionManager>,
        player_uuids: &[String],
        message: &WebSocketMessage,
    ) -> Result<(), RoomEventError> {
        let message_json = serde_json::to_string(message).map_err(|e| {
            RoomEventError::HandlerError(format!("Failed to serialize message: {}", e))
        })?;

        for uuid in player_uuids {
            connection_manager.send_to_player(uuid, &message_json).await;
        }

        Ok(())
    }

    #[allow(dead_code)] // Alternative broadcast method for UUID-based messaging
    pub async fn broadcast_to_room_via_uuids(
        connection_manager: &Arc<dyn ConnectionManager>,
        player_uuids: &[String],
        message_json: &str,
    ) {
        for uuid in player_uuids {
            connection_manager.send_to_player(uuid, message_json).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockConnectionManager {
        sent: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl ConnectionManager for MockConnectionManager {
        async fn add_connection(&self, _uuid: String, _sender: mpsc::Sender<String>) {}
        async fn remove_connection(&self, _uuid: &str) {}
        async fn send_to_player(&self, uuid: &str, message: &str) {
            self.sent
                .lock()
                .unwrap()
                .push((uuid.to_string(), message.to_string()));
        }
        async fn send_to_players(&self, uuids: &[String], message: &str) {
            for u in uuids {
                self.send_to_player(u, message).await;
            }
        }
        async fn count_online_players(&self) -> usize {
            0
        }
    }

    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_broadcast_to_players_serializes_and_sends() {
        let mgr_concrete = Arc::new(MockConnectionManager {
            sent: std::sync::Mutex::new(vec![]),
        });
        let mgr: Arc<dyn ConnectionManager> = mgr_concrete.clone();

        let players = vec!["p1".to_string(), "p2".to_string()];
        let msg = WebSocketMessage::error("oops".to_string());

        MessageBroadcaster::broadcast_to_players(&mgr, &players, &msg)
            .await
            .unwrap();

        let sent = mgr_concrete.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, "p1");
        assert_eq!(sent[1].0, "p2");

        for (_uuid, body) in sent {
            // Should be valid JSON matching message
            let parsed: WebSocketMessage = serde_json::from_str(&body).unwrap();
            assert_eq!(
                parsed.message_type,
                crate::websockets::messages::MessageType::Error
            );
        }
    }

    #[tokio::test]
    async fn test_broadcast_to_room_via_uuids_uses_raw_json() {
        let mgr_concrete = Arc::new(MockConnectionManager {
            sent: std::sync::Mutex::new(vec![]),
        });
        let mgr: Arc<dyn ConnectionManager> = mgr_concrete.clone();
        let players = vec!["p1".to_string()];
        let json = "{\"type\":\"ERROR\",\"payload\":{\"message\":\"x\"},\"meta\":null}";

        MessageBroadcaster::broadcast_to_room_via_uuids(&mgr, &players, json).await;

        let sent = mgr_concrete.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "p1");
        assert_eq!(sent[0].1, json);
    }
}
