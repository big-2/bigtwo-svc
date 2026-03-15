# AGENTS.md

This file defines strict rules for automated coding agents. Follow rules exactly. Prefer minimal changes over large rewrites.

This file provides guidance to agents when working with code in this repository.

## Project Overview

This is the Rust backend for Big Two, a real-time multiplayer card game. Built with axum + tokio, it features event-driven architecture, WebSocket communication, and complete Big Two gameplay mechanics.

## Development Commands

```bash
# Development (auto-detects PostgreSQL or falls back to in-memory)
./scripts/dev.sh --postgres    # Force PostgreSQL
./scripts/dev.sh --memory      # Force in-memory storage

# Build and test
cargo check                    # Fast compile check
cargo test                     # Run unit tests
cargo test -- --ignored        # Run integration tests (requires DB)
cargo test test_name -- --nocapture  # Run single test with output
cargo clippy                   # Lint (must pass with no warnings)
cargo clippy -- -D warnings    # Treat warnings as errors
cargo fmt                      # Format code
cargo fmt -- --check           # Check formatting without modifying

# Database (when using PostgreSQL)
sqlx migrate run              # Apply migrations
sqlx migrate add <name>       # Create migration

# Session endpoint testing
./scripts/test-session.sh     # Test REST endpoints
```

**CRITICAL**: After ANY code change, the agent MUST run:
1. `cargo check`
2. `cargo test`
3. `cargo clippy`
4. `cargo fmt`

The task is NOT complete unless all succeed.

## Architecture Overview

Architecture rules:

- Components must communicate through `EventBus`
- Game logic must not call WebSocket code directly
- Services contain business logic
- Repositories only store data
- Handlers must not contain game logic

Configuration behavior:

- Use PostgreSQL if `DATABASE_URL` is set
- Otherwise fall back to in-memory storage

## Directory Structure

```
src/
├── main.rs                   # Entry point, axum server setup, dependency injection
├── lib.rs                    # Public API for integration tests
├── shared.rs                 # AppState, AppError, test utilities
├── event/                    # Owns the event bus and event definitions. Cross-component communication belongs here.
│   ├── bus.rs               # EventBus implementation
│   ├── events.rs            # RoomEvent definitions
│   ├── room_handler.rs      # Event handler trait
│   └── room_subscription.rs # Room-specific event subscriptions
├── session/                  # Owns authentication and session lifecycle. Session-related business logic belongs here.
│   ├── handlers.rs          # REST endpoints: create, validate
│   ├── middleware.rs        # JWT authentication middleware
│   ├── repository.rs        # In-memory + PostgreSQL implementations
│   ├── service.rs           # Business logic
│   ├── creator.rs           # Session creation orchestrator with transaction-like semantics
│   ├── generators.rs        # Username generator trait and implementations
│   ├── token.rs             # JWT utilities
│   └── models.rs, types.rs  # Data structures
├── room/                     # Owns room lifecycle. Room creation, join, leave, and room metadata belong here.
│   ├── handlers.rs          # REST endpoints: create, join, list, get
│   ├── repository.rs        # In-memory storage (uses pet-name IDs)
│   ├── service.rs           # Business logic
│   └── models.rs, types.rs  # Data structures
├── game/                     # Owns game rules and state. Only this module may modify game state.
│   ├── cards/               # Card system
│   │   ├── basic.rs        # Card types, Big Two sorting rules
│   │   └── hands.rs        # Hand validation and comparison
│   ├── core.rs              # Core game rules, turn progression
│   ├── repository.rs        # Game state repository
│   ├── service.rs           # Game service layer
│   └── game_room_subscriber.rs # Event handler for game logic
├── websockets/               # Owns realtime messaging only. Must not contain game rules or business logic.
│   ├── handler.rs           # WebSocket upgrade and message routing
│   ├── messages.rs          # Message type definitions
│   ├── event_handlers/      # Organized event handling
│   │   ├── chat_events.rs  # Chat message handling
│   │   ├── game_events.rs  # Game move handling
│   │   ├── room_events.rs  # Room lifecycle events
│   │   ├── connection_events.rs # Connection/disconnection
│   │   └── shared/         # Shared utilities (player mapping, broadcasts)
│   ├── connection_manager.rs # Per-room connection tracking
│   ├── socket.rs            # Individual WebSocket handling
│   └── websocket_room_subscriber.rs # Event handler for WebSocket broadcast
├── bot/                      # Owns AI player lifecycle and bot move generation.
│   ├── manager.rs           # Bot lifecycle management
│   ├── basic_strategy.rs    # Basic bot playing strategy
│   ├── strategy_factory.rs  # Factory for creating strategies by difficulty level
│   ├── bot_room_subscriber.rs # Bot event handling
│   ├── handlers.rs          # REST endpoints for bot operations
│   └── types.rs             # Bot-related types (BotDifficulty, BotStrategy trait)
├── stats/                    # Owns statistics collection and score calculation. Stats updates belong here via events.
│   ├── models.rs            # Data structures (GameResult, RoomStats, PlayerStats)
│   ├── service.rs           # Stats service and room subscriber
│   ├── repository.rs        # Stats storage (in-memory with per-room locking)
│   ├── errors.rs            # Stats-specific error types (StatsError)
│   ├── collectors/          # Data collectors (cards remaining, win/loss)
│   └── calculators/         # Score calculators (card count, 10+ multiplier)
└── user/                     # User management
    └── mapping_service.rs   # Player ID to username mapping
```

## Key Components

### AppState (shared.rs)
Central dependency injection container holding all repositories, services, managers, and the event bus. Contains builder pattern for testing.

### SessionCreator (session/creator.rs)
Orchestrates the complex session creation process with transaction-like semantics. Coordinates username generation, session storage, player mapping, and JWT token creation. Supports configurable session expiration (default 365 days, configurable via SESSION_EXPIRATION_DAYS env var).

### EventBus (event/)
Central message broker enabling decoupled communication. Supports both global and room-specific event subscriptions. Key event types include game moves, player connections/disconnections, and room lifecycle events.

### GameService (game/service.rs)
Manages Big Two game state per room. Handles game creation, move validation, turn progression, and win detection. Uses event system for communication.

### ConnectionManager (websockets/connection_manager.rs)
Tracks WebSocket connections per room for message broadcasting. Manages connection lifecycle and message routing.

### BotManager (bot/manager.rs)
Manages AI bot players in rooms. Handles bot creation, move generation, and lifecycle. Bots use basic strategy to play valid moves.

### StatsService (stats/service.rs)
Tracks game statistics per room. Uses collector pattern for data gathering and calculator pattern for score computation. Automatic reset when room empties.

### Repository Pattern
- **SessionRepository**: JWT session storage (in-memory or PostgreSQL)
- **RoomRepository**: Game room management (in-memory only)
- **StatsRepository**: Per-room statistics (in-memory with per-room locking)

## Big Two Game Rules

These rules are authoritative. Do not change card ordering or rules unless requested.

- **Card Order**: 3 < 4 < 5 < 6 < 7 < 8 < 9 < 10 < J < Q < K < A < 2 (2 is highest)
- **Suit Order**: Diamonds < Clubs < Hearts < Spades
- **Format**: "3D", "KH", "AS" (rank + suit)
- **First Move**: Must include 3 of Diamonds

## API Endpoints

API endpoints are considered stable. Do not modify routes or message formats unless explicitly requested.

### REST (session-based auth via X-Session-ID header)
- `POST /session` - Create session with auto-generated username
- `GET /session/validate` - Validate session (authenticated)
- `POST /room` - Create room (returns pet-name ID)
- `GET /rooms` - List all rooms
- `GET /room/{id}` - Get room details
- `GET /room/{id}/stats` - Get current stats for room (games played, player stats)
- `POST /room/{id}/join` - Join room (authenticated)
- `DELETE /room/{id}` - Delete room (host only)
- `POST /room/{id}/bot/add` - Add AI bot to room
- `DELETE /room/{id}/bot/{bot_uuid}` - Remove bot from room

### WebSocket
- `GET /ws/{room_id}` - Real-time game communication (JWT auth via `Sec-WebSocket-Protocol` header)
- Message types: `CHAT`, `MOVE`, `LEAVE`, `START_GAME`, `READY` (client→server)
- Message types: `PLAYERS_LIST`, `MOVE_PLAYED`, `TURN_CHANGE`, `GAME_STARTED`, `GAME_WON`, `GAME_RESET`, `BOT_ADDED`, `BOT_REMOVED`, `STATS_UPDATED`, `ERROR`, `HOST_CHANGE` (server→client)

## Testing

Do not modify tests unless the task explicitly requires it. Tests define expected behavior.

### Structure
```
tests/
├── websocket_workflow_tests.rs  # Full game scenario integration tests
├── stats_room_subscriber_tests.rs # Stats subscriber event handling tests
└── utils/                       # Test utilities and helpers
    ├── setup.rs                 # AppState and test environment setup
    ├── mocks.rs                 # Mock repositories and managers
    ├── game_builders.rs         # Helper functions for game scenarios
    ├── actions.rs               # Common test actions (join room, make move)
    └── assertions.rs            # Game state assertions
```

### Test Categories
- **Unit tests**: Mock repositories, no external dependencies
- **Integration tests**: Use test database, marked with `#[ignore]`
- **Workflow tests**: Full game scenarios from start to finish

### Running Tests
```bash
cargo test                    # Unit tests only
cargo test -- --ignored      # Integration tests (requires DB)
cargo test test_name -- --nocapture  # Single test with output
```

## Event-Driven Flow

Rules:

- Incoming messages must go through handler -> event -> subscriber
- Subscribers perform logic
- WebSocket subscriber performs broadcasting
- Do not bypass `EventBus`

Flow:

1. `websockets/handler.rs` receives the incoming message
2. The handler emits a `RoomEvent` through `EventBus`
3. Subscribers react to the event
4. `websockets/websocket_room_subscriber.rs` broadcasts outbound messages

## Code Quality Requirements

**CRITICAL**: After ANY code change, the agent MUST run:
1. **`cargo check`**
2. **`cargo test`**
3. **`cargo clippy`**
4. **`cargo fmt`**

The task is NOT complete unless all succeed.

**Code Style Guidelines**:
- **Idiomatic Rust**: Use `Result`, `Option`, `?`, traits, proper error handling
- **No unsafe code**: Use clippy-friendly patterns
- **Event-driven patterns**: Emit events through EventBus rather than direct function calls
- **Dependency injection**: Use traits for services to enable mocking in tests
- **Only modify code required for the task**: Do not refactor unrelated code
- **Naming stability**: Do not rename functions unless required
- **File stability**: Do not move files unless required

## Agent Rules

These rules are for autonomous coding agents working in this repository:

- Prefer minimal changes
- Do not refactor unrelated code
- Do not introduce new dependencies unless required
- Do not change public APIs unless requested
- Follow existing patterns in the repo
- Search the codebase before implementing new patterns
- Ensure code compiles and tests pass before finishing

## How to Navigate the Codebase

Agents should understand the architecture before making changes:

- Start from `main.rs`
- `AppState` holds dependencies
- `EventBus` connects subsystems
- Game logic lives in `game/`
- WebSocket handlers must not contain game logic
- Repositories store data only
- Services contain business logic

## Change Strategy

When implementing changes, follow this sequence:

1. Identify the correct module
2. Prefer adding events instead of direct calls
3. Update repository only if persistence is needed
4. Update service for business logic
5. Add tests if behavior changes
6. Run `cargo check` / `cargo test` / `cargo clippy` / `cargo fmt`

## Event System Rules

These rules are especially important in this repository:

- All cross-component communication goes through `EventBus`
- Game logic must not call WebSocket code directly
- WebSocket handlers must emit events
- New features should use events instead of direct calls
- Subscribers handle side effects

## Definition of Done

A change is complete only when all of the following are true:

- Code compiles
- All tests pass
- No clippy warnings
- Code formatted
- No unrelated files changed
- Architecture rules respected

## Testing Requirements for Agents

Agents are expected to run these commands before finishing:

- Always run `cargo check`
- Always run `cargo test`
- Always run `cargo clippy`
- Always run `cargo fmt`
- Do not finish the task if any fail

## When Not to Modify Code

Avoid over-editing. Do not make code changes in these cases:

- Do not modify code if the request is unclear
- Do not rewrite large sections unnecessarily
- Do not introduce new architecture
- Do not change multiple subsystems without reason
- Ask for clarification instead of guessing, if the environment supports it

## Component Ownership

Use this mapping to decide where changes belong:

- `session/` -> authentication
- `room/` -> room lifecycle
- `game/` -> game rules and state
- `websockets/` -> realtime messaging only
- `bot/` -> AI players
- `stats/` -> statistics
- `event/` -> event bus

## Example Change Pattern

Example workflow for adding a new game event:

1. Add event to `event/events.rs`
2. Emit event from service
3. Handle event in subscriber
4. Broadcast via WebSocket subscriber
5. Add tests

## Concurrency Rules

This project uses Tokio and async Rust. Follow these rules:

- Do not block async tasks
- Avoid holding locks across `await`
- Use existing `Arc` / `Mutex` patterns
- Follow current async style

## Database Rules

Follow these constraints for persistence changes:

- Do not change schema without a migration
- Use `sqlx migrate`
- Keep repository traits compatible
- Do not hardcode DB logic in services

## Goal

The goal of these instructions is to:

- Reduce agent hallucinated refactors
- Enforce architecture consistency
- Make autonomous edits safe
- Improve agentic coding performance
- Ensure minimal, correct, test-passing changes

## Bot System

The backend includes AI bots for testing and single-player gameplay:
- **Bot creation**: Add bots via REST endpoint `POST /room/{id}/bot/add`
- **Bot strategies**: Multiple difficulty levels (Easy, Medium, Hard) supported via BotStrategyFactory
  - Currently all levels use BasicBotStrategy
  - Medium and Hard strategies are not implemented yet
- **Strategy pattern**: BotStrategy trait allows easy addition of new bot behaviors
- **Event-driven**: Bots respond to `TurnChange` events with automatic moves
- **Integration**: Bots appear as regular players to other clients

Do not add new strategies unless requested.

## Stats System

The backend tracks game statistics per room:
- **Data collectors**: `CardsRemainingCollector`, `WinLossCollector` gather game data
- **Score calculators**: `CardCountScoreCalculator`, `TenPlusMultiplierCalculator` compute scores with priorities
- **Per-room locking**: Thread-safe statistics updates using per-room mutexes
- **Automatic reset**: Stats reset when room becomes empty (no human players)
- **Bot filtering**: Bots are excluded from statistics tracking
- **REST endpoint**: `GET /room/{id}/stats` to fetch current statistics
- **WebSocket updates**: `STATS_UPDATED` message broadcast after each game

Stats system must only update via events. Game logic must not modify stats directly.
