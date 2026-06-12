// Public API
#[allow(unused_imports)]
// Used by integration tests and external callers implementing ConnectionManager.
pub use connection_manager::{ConnectionManager, ConnectionToken, InMemoryConnectionManager};
pub use handler::websocket_handler;
#[allow(unused_imports)] // Used by integration tests
pub use handler::WebsocketReceiveHandler;
#[allow(unused_imports)] // Used by integration tests
pub use messages::{MessageType, WebSocketMessage};
#[allow(unused_imports)] // Used by integration tests
pub use socket::MessageHandler;
pub use websocket_room_subscriber::WebSocketRoomSubscriber;

// Internal modules
mod connection_manager;
pub mod event_handlers;
mod handler;
mod messages;
mod socket;
mod websocket_room_subscriber;
