// Public API
#[allow(unused_imports)] // Used by integration tests
pub use cards::SingleHand;
pub use cards::{Card, Hand, Rank, Suit};
#[allow(unused_imports)] // Used by integration tests
pub use core::Player;
#[allow(unused_imports)] // Public re-export for downstream consumers
pub use core::{Game, MoveAction, MoveRecord};
pub use game_room_subscriber::GameEventRoomSubscriber;
pub use service::GameService;

// Internal modules
mod cards;
mod core;
mod game_room_subscriber;
mod repository;
mod service;
