// Public API - what other modules can use
pub use handlers::{
    create_room, get_current_room, get_room_details, get_room_stats, join_room, leave_room,
    list_rooms,
};

// Internal modules
pub mod activity_room_subscriber;
pub mod activity_tracker;
pub mod cleanup_task;
mod handlers;
pub mod models;
pub mod repository;
pub mod service;
mod types;
