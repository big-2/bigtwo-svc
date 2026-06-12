use serde::{Deserialize, Serialize};

/// External request payload for creating a new room (no data needed)
#[derive(Debug, Deserialize)]
pub struct CreateRoomApiRequest {
    // Currently empty - room creation is based on authenticated session
}

/// Internal request payload for creating a new room
#[derive(Debug, Deserialize)]
pub struct RoomCreateRequest {
    pub host_uuid: String, // Populated from authenticated session
}

/// Request payload for joining a room
/// Currently empty since player info comes from session,
/// but allows for future expansion (e.g., join as spectator)
#[derive(Debug, Deserialize)]
pub struct JoinRoomRequest {
    // Future fields might include:
    // pub join_as_spectator: Option<bool>,
}

/// Response for room creation and room information
#[derive(Debug, Serialize, Deserialize)]
pub struct RoomResponse {
    pub id: String,
    pub host_name: String,
    pub status: String,
    pub player_count: i32,
}

/// Authenticated response for the current user's room membership.
#[derive(Debug, Serialize, Deserialize)]
pub struct CurrentRoomResponse {
    pub room: RoomResponse,
    pub is_connected: bool,
    pub has_active_game: bool,
}

/// Response for explicit room leave requests.
#[derive(Debug, Serialize, Deserialize)]
pub struct LeaveRoomResponse {
    pub status: String,
}
