use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::debug;

use super::events::RoomEvent;

/// Event bus for distributing events throughout the application
#[derive(Debug, Clone)]
pub struct EventBus {
    /// Room-specific event channels: room_id -> sender
    room_channels: Arc<RwLock<HashMap<String, broadcast::Sender<RoomEvent>>>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// Creates a new event bus with the specified room capacity
    pub fn new() -> Self {
        Self {
            room_channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Emits an event to all subscribers of a specific room
    pub async fn emit_to_room(&self, room_id: &str, event: RoomEvent) {
        let room_channels = self.room_channels.read().await;

        if let Some(sender) = room_channels.get(room_id) {
            match sender.send(event.clone()) {
                Ok(receiver_count) => {
                    debug!(
                        room_id = %room_id,
                        receivers = receiver_count,
                        event = ?event,
                        "Room event emitted"
                    );
                }
                Err(_) => {
                    debug!(room_id = %room_id, "Room event emitted with no receivers");
                }
            }
        } else {
            debug!(room_id = %room_id, ?event, "Dropping room event with no room channel");
        }
    }

    /// Subscribe to events for a specific room
    pub async fn subscribe_to_room(&self, room_id: &str) -> broadcast::Receiver<RoomEvent> {
        let room_channels = self.room_channels.read().await;

        if let Some(sender) = room_channels.get(room_id) {
            sender.subscribe()
        } else {
            debug!(room_id = %room_id, "Creating new room channel for subscription");
            drop(room_channels);

            // Create room channel if it doesn't exist
            let mut room_channels = self.room_channels.write().await;
            let (sender, _) = broadcast::channel(100); // Room capacity
            let receiver = sender.subscribe();
            room_channels.insert(room_id.to_string(), sender);
            receiver
        }
    }

    /// Remove the event channel for a room and close all room receivers.
    pub async fn remove_room(&self, room_id: &str) {
        let mut room_channels = self.room_channels.write().await;
        if room_channels.remove(room_id).is_some() {
            debug!(room_id = %room_id, "Removed room event channel");
        } else {
            debug!(room_id = %room_id, "No room event channel found to remove");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn removing_room_closes_existing_receivers() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe_to_room("room-1").await;

        bus.remove_room("room-1").await;

        let result = rx.recv().await;
        assert!(matches!(result, Err(broadcast::error::RecvError::Closed)));
    }

    #[tokio::test]
    async fn emit_does_not_recreate_removed_room_channel() {
        let bus = EventBus::new();
        let _rx = bus.subscribe_to_room("room-1").await;
        bus.remove_room("room-1").await;

        bus.emit_to_room(
            "room-1",
            RoomEvent::PlayerJoined {
                player: "player-1".to_string(),
            },
        )
        .await;

        let mut rx = bus.subscribe_to_room("room-1").await;
        let result = tokio::time::timeout(std::time::Duration::from_millis(20), rx.recv()).await;

        assert!(result.is_err());
    }
}
