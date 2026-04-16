mod bot;
mod event;
mod game;
mod player;
mod room;
mod session;
mod shared;
mod stats;
mod user;
mod websockets;

use axum::{
    http::{HeaderValue, Method},
    middleware,
    routing::{get, post},
    Router,
};
use room::repository::InMemoryRoomRepository;
use room::service::RoomService;
use session::repository::{
    InMemorySessionRepository, PostgresSessionRepository, SessionRepository,
};
use session::service::SessionService;
use shared::AppState;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::bot::BotManager;
use crate::stats::{
    service::StatsService, InMemoryGameHistoryRepository, InMemoryStatsRepository,
    PostgresGameHistoryRepository,
};
use crate::websockets::InMemoryConnectionManager;
use crate::{
    event::EventBus,
    game::{repository::PostgresGameRepository, GameService},
    user::mapping_service::InMemoryPlayerMappingService,
};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bigtwo=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Big Two game server");

    if let Err(config_error) = bot::ai_strategy::validate_ai_bot_service_configuration() {
        error!(
            error = %config_error,
            "Invalid AI bot service configuration"
        );
        std::process::exit(1);
    }

    // Create shared application state with dependency injection
    // Smart configuration: Use PostgreSQL if DATABASE_URL is set, otherwise in-memory
    let postgres_pool = match std::env::var("DATABASE_URL") {
        Ok(database_url) => match sqlx::PgPool::connect(&database_url).await {
            Ok(pool) => {
                info!("✅ Connected to PostgreSQL successfully");
                Some(pool)
            }
            Err(e) => {
                warn!("❌ Failed to connect to PostgreSQL: {}", e);
                None
            }
        },
        Err(e) => {
            info!("Error getting DATABASE_URL: {}", e);
            info!("Using in-memory session storage (sessions lost on restart)");
            info!("💡 Set DATABASE_URL to use PostgreSQL for persistent sessions");
            None
        }
    };

    let session_repository: Arc<dyn SessionRepository + Send + Sync> =
        if let Some(pool) = &postgres_pool {
            info!("Using PostgreSQL session storage (persistent across restarts)");
            Arc::new(PostgresSessionRepository::new(pool.clone()))
        } else {
            info!("🔄 Falling back to in-memory session storage");
            Arc::new(InMemorySessionRepository::new())
        };

    let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
    let session_service = Arc::new(SessionService::new(
        session_repository.clone(),
        player_mapping.clone(),
    ));
    let room_repository = Arc::new(InMemoryRoomRepository::new());
    let event_bus = EventBus::new();
    let connection_manager = Arc::new(InMemoryConnectionManager::new());
    let game_service = if let Some(pool) = &postgres_pool {
        info!("Using PostgreSQL active game storage (persistent across restarts)");
        Arc::new(GameService::with_repository(
            Arc::new(PostgresGameRepository::new(pool.clone())),
            player_mapping.clone(),
        ))
    } else {
        info!("🔄 Falling back to in-memory active game storage");
        Arc::new(GameService::new(player_mapping.clone()))
    };
    let bot_manager = Arc::new(BotManager::new());

    // Stats system: in-memory tracking of per-room game statistics
    info!("📊 Stats tracking enabled (in-memory)");
    let stats_repository = Arc::new(InMemoryStatsRepository::new());
    let mut stats_service_builder =
        StatsService::builder(stats_repository.clone()).with_bot_manager(bot_manager.clone());
    if let Some(pool) = &postgres_pool {
        info!("🗂️ Completed game history persistence enabled (PostgreSQL)");
        stats_service_builder = stats_service_builder.with_game_history_repository(Arc::new(
            PostgresGameHistoryRepository::new(pool.clone()),
        ));
    } else {
        info!("🗂️ Completed game history persistence enabled (in-memory)");
        stats_service_builder = stats_service_builder
            .with_game_history_repository(Arc::new(InMemoryGameHistoryRepository::new()));
    }
    let stats_service = Arc::new(stats_service_builder.build());
    // Create RoomService focused purely on business logic
    let room_service = Arc::new(RoomService::new(room_repository.clone()));

    // Activity tracking: monitors room events and updates activity timestamps
    let activity_tracker = Arc::new(room::activity_tracker::ActivityTracker::new(
        room_repository.clone(),
    ));
    let activity_subscriber = Arc::new(
        room::activity_room_subscriber::ActivityRoomSubscriber::new(activity_tracker),
    );

    // Spawn background cleanup task for inactive rooms
    let cleanup_config = room::cleanup_task::CleanupConfig::default();
    tokio::spawn(room::cleanup_task::start_cleanup_task(
        room_repository.clone(),
        game_service.clone(),
        Arc::new(event_bus.clone()),
        bot_manager.clone(),
        player_mapping.clone(),
        stats_service.clone(),
        cleanup_config,
    ));

    let session_cleanup_config = session::cleanup_task::CleanupConfig::default();
    tokio::spawn(session::cleanup_task::start_cleanup_task(
        session_service.clone(),
        session_cleanup_config,
    ));

    let app_state = AppState::builder()
        .with_session_repository(session_repository)
        .with_session_service(session_service)
        .with_room_service(room_service)
        .with_event_bus(event_bus)
        .with_activity_subscriber(activity_subscriber)
        .with_connection_manager(connection_manager)
        .with_game_service(game_service)
        .with_player_mapping(player_mapping)
        .with_bot_manager(bot_manager)
        .with_stats_repository(stats_repository)
        .with_stats_service(stats_service)
        .build()
        .expect("Failed to build AppState - all required dependencies should be provided");

    // Configure CORS for development and production
    // Allow origins from environment variable for production, fallback to localhost for dev
    // Set ALLOWED_ORIGINS="*" to allow all origins (dev/staging only!)
    let allowed_origins = std::env::var("ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "https://localhost:5175,http://localhost:5175,https://localhost:5173,http://localhost:5173".to_string());

    let cors = if allowed_origins.trim() == "*" {
        info!("⚠️  CORS: Allowing ALL origins (dev mode - insecure for production!)");
        CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods([Method::GET, Method::POST, Method::DELETE])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ])
    } else {
        let origins: Vec<HeaderValue> = allowed_origins
            .split(',')
            .filter_map(|origin| origin.trim().parse::<HeaderValue>().ok())
            .collect();
        info!("Allowed CORS origins: {:?}", origins);
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST, Method::DELETE])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ])
    };

    // build our application with a single route
    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/health", get(|| async { "OK" }))
        .route("/online", get(session::get_online_count))
        .route(
            "/player/me/stats",
            get(player::get_my_stats).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route(
            "/player/me/games",
            get(player::get_my_recent_games).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route(
            "/games/:game_id",
            get(player::get_completed_game).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route("/session", post(session::create_session))
        .route(
            "/session/validate",
            get(session::validate_session).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route(
            "/room",
            post(room::create_room).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route("/rooms", get(room::list_rooms))
        .route(
            "/room/:room_id/join",
            post(room::join_room).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route("/room/:room_id", get(room::get_room_details))
        .route("/room/:room_id/stats", get(room::get_room_stats))
        .route(
            "/room/:room_id/bot",
            post(bot::handlers::add_bot_to_room).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route(
            "/room/:room_id/bot/:bot_uuid",
            axum::routing::delete(bot::handlers::remove_bot_from_room).layer(
                middleware::from_fn_with_state(app_state.clone(), session::jwt_auth),
            ),
        )
        .route("/ws/:room_id", get(websockets::websocket_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    // Run our app with hyper, listening globally on configured port
    // Railway provides PORT env var, default to 3000 for local development
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("Server running on {}", addr);
    axum::serve(listener, app).await.unwrap();
}
