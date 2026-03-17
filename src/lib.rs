// Library crate for Big Two game server
// This file exposes the public API for integration tests

pub mod bot;
pub mod event;
pub mod game;
pub mod player;
pub mod room;
pub mod session;
pub mod shared;
pub mod stats;
pub mod user;
pub mod websockets;

// Re-export commonly used types for easier access in tests
// Note: Some re-exports are only used by integration tests (tests/ directory),
// so clippy may warn they're unused when checking just the lib.
#[allow(unused_imports)]
pub use bot::{BotManager, BotRoomSubscriber};
pub use event::{EventBus, RoomEvent, RoomSubscription};
pub use game::GameService;
pub use room::{models::RoomModel, repository::RoomRepository};
pub use shared::AppError;
#[allow(unused_imports)]
pub use stats::{
    models::*, repository::InMemoryStatsRepository, repository::StatsRepository,
    StatsRoomSubscriber, StatsService,
};
pub use user::PlayerMappingService;
pub use websockets::{ConnectionManager, WebSocketRoomSubscriber};
