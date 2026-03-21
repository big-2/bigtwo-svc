use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use super::{bus::EventBus, room_handler::RoomEventHandler};

/// Manages room event subscriptions and routes events to handlers
pub struct RoomSubscription {
    room_id: String,
    handler: Arc<dyn RoomEventHandler>,
    event_bus: EventBus,
}

impl RoomSubscription {
    pub fn new(room_id: String, handler: Arc<dyn RoomEventHandler>, event_bus: EventBus) -> Self {
        Self {
            room_id,
            handler,
            event_bus,
        }
    }

    /// Start the subscription - spawns a background task that listens to room events
    /// and routes them to the handler
    pub async fn start(self) -> JoinHandle<()> {
        let room_id = self.room_id.clone();
        let handler_name = self.handler.handler_name();

        debug!(
            room_id = %room_id,
            handler = handler_name,
            "Starting room subscription"
        );

        let mut receiver = self.event_bus.subscribe_to_room(&room_id).await;

        tokio::spawn(async move {
            debug!(
                room_id = %room_id,
                handler = handler_name,
                "Room subscription task started"
            );

            while let Ok(event) = receiver.recv().await {
                debug!(
                    room_id = %room_id,
                    handler = handler_name,
                    event = ?event,
                    "Received room event"
                );

                if let Err(e) = self.handler.handle_room_event(&room_id, event).await {
                    warn!(
                        room_id = %room_id,
                        handler = handler_name,
                        error = %e,
                        "Room event handler failed"
                    );
                }
            }

            warn!(
                room_id = %room_id,
                handler = handler_name,
                "Room subscription ended - no more events"
            );
        })
    }
}
