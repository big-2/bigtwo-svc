use std::sync::Arc;
use tracing::{info, warn};

use crate::{
    event::RoomEventError,
    room::service::RoomService,
    websockets::{connection_manager::ConnectionManager, messages::WebSocketMessage},
};

use super::shared::{MessageBroadcaster, RoomQueryUtils};

pub struct ChatEventHandlers {
    room_service: Arc<RoomService>,
    connection_manager: Arc<dyn ConnectionManager>,
}

impl ChatEventHandlers {
    pub fn new(
        room_service: Arc<RoomService>,
        connection_manager: Arc<dyn ConnectionManager>,
    ) -> Self {
        Self {
            room_service,
            connection_manager,
        }
    }

    pub async fn handle_chat_message(
        &self,
        room_id: &str,
        sender_uuid: &str,
        content: &str,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            sender_uuid = %sender_uuid,
            "Handling chat message event"
        );

        let room = match RoomQueryUtils::get_room_if_exists(&self.room_service, room_id).await? {
            Some(room) => room,
            None => {
                warn!(room_id = %room_id, "Room was deleted, no chat notifications needed");
                return Ok(());
            }
        };

        let chat_message = WebSocketMessage::chat(sender_uuid.to_string(), content.to_string());
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            room.get_player_uuids(),
            &chat_message,
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::{
        models::RoomModel, repository::InMemoryRoomRepository, repository::RoomRepository,
    };

    struct CollectingConnMgr(std::sync::Mutex<Vec<(String, String)>>);

    #[async_trait::async_trait]
    impl ConnectionManager for CollectingConnMgr {
        async fn add_connection(
            &self,
            _uuid: String,
            _sender: mpsc::Sender<String>,
        ) -> crate::websockets::ConnectionToken {
            crate::websockets::ConnectionToken(0)
        }
        async fn remove_connection(&self, _uuid: &str) {}
        async fn remove_connection_if_current(
            &self,
            _uuid: &str,
            _token: crate::websockets::ConnectionToken,
        ) -> bool {
            true
        }
        async fn send_to_player(&self, uuid: &str, message: &str) {
            self.0
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
    async fn test_handle_chat_message_broadcasts_to_room_players() {
        let repo = Arc::new(InMemoryRoomRepository::new());
        let now = chrono::Utc::now();
        let room = RoomModel {
            id: "r1".into(),
            host_uuid: Some("h".into()),
            status: "ONLINE".into(),
            player_uuids: vec!["a".into(), "b".into()],
            ready_players: vec![],
            connected_players: vec!["a".into(), "b".into()],
            created_at: now,
            last_activity_at: now,
        };
        repo.create_room(&room).await.unwrap();
        let room_service = Arc::new(RoomService::new(repo));

        let mgr_concrete = Arc::new(CollectingConnMgr(std::sync::Mutex::new(vec![])));
        let mgr: Arc<dyn ConnectionManager> = mgr_concrete.clone();
        let handler = ChatEventHandlers::new(room_service, mgr.clone());

        handler.handle_chat_message("r1", "a", "hi").await.unwrap();

        let sent = mgr_concrete.0.lock().unwrap().clone();

        assert_eq!(sent.len(), 2);
        assert!(sent.iter().any(|(u, _)| u == "a"));
        assert!(sent.iter().any(|(u, _)| u == "b"));
    }
}
