# Big Two - Multiplayer Card Game

A real-time multiplayer implementation of Big Two (大老二), a popular Chinese card game. Built with Rust backend and TypeScript React frontend, featuring AI bots, game statistics, and event-driven architecture.

**Play now: [https://big2.app](https://big2.app)**

## Features

- **Real-time Multiplayer**: WebSocket-based gameplay supporting up to 4 players
- **AI Bot System**: Configurable bot players with multiple difficulty levels
- **Game Statistics**: Automatic tracking of wins, losses, scores, and streaks
- **Session Management**: JWT-based authentication with configurable expiration (default 365 days)
- **Event-Driven Architecture**: Decoupled components using EventBus pattern
- **Flexible Storage**: PostgreSQL for persistence or in-memory for fast development
- **Complete Big Two Rules**: Full implementation of card rankings, hand validation, and game flow

## Tech Stack

### Backend (Rust)
- **axum** - Web framework
- **tokio** - Async runtime
- **sqlx** - Database driver (PostgreSQL)
- **tower** - Middleware
- **serde** - Serialization
- **jsonwebtoken** - JWT authentication

### Frontend (TypeScript + React)
- **React 18** with TypeScript
- **Vite** - Build tool
- **Tailwind CSS** + **shadcn/ui** - Styling and components
- **WebSocket** - Real-time communication

## Quick Start

### Prerequisites
- Rust 1.70+ and Cargo
- Node.js 18+ and npm
- PostgreSQL (optional, can use in-memory storage)

### Backend Setup

```bash
cd bigtwo

# Development with in-memory storage (fastest, no DB required)
./scripts/dev.sh --memory

# Development with PostgreSQL (persistent sessions)
export DATABASE_URL="postgres://user:pass@localhost/bigtwo"
./scripts/dev.sh --postgres

# Run tests
cargo test                    # Unit tests
cargo test -- --ignored      # Integration tests (requires DB)

# Code quality checks
cargo clippy                  # Lint
cargo fmt                     # Format
```

### Frontend Setup

```bash
cd bigtwo-ui

npm install
npm run dev                   # Development server
npm run build                 # Production build
```

## Architecture

### Event-Driven Design
The backend uses an EventBus for decoupled communication between components:
- **GameRoomSubscriber**: Handles game logic and rule enforcement
- **WebSocketRoomSubscriber**: Broadcasts events to connected clients
- **BotRoomSubscriber**: Manages AI bot responses
- **StatsRoomSubscriber**: Tracks game statistics

### Repository Pattern
Abstractions for data storage enable easy switching between storage backends:
- **SessionRepository**: User sessions (in-memory or PostgreSQL)
- **RoomRepository**: Game rooms (in-memory)
- **StatsRepository**: Game statistics (in-memory with per-room locking)

### Directory Structure
```
src/
├── main.rs              # Server setup and dependency injection
├── event/               # EventBus and event definitions
├── session/             # JWT authentication and session management
├── room/                # Game room lifecycle management
├── game/                # Big Two game logic and card system
├── websockets/          # WebSocket handling and message routing
├── bot/                 # AI bot system with strategy pattern
├── stats/               # Game statistics tracking
└── user/                # Player mapping service
```

## API Documentation

### REST Endpoints

**Session Management**
- `POST /session` - Create new session with auto-generated username
- `GET /session/validate` - Validate session (requires X-Session-ID header)

**Room Management**
- `POST /room` - Create room (returns pet-name ID)
- `GET /rooms` - List all rooms
- `GET /room/{id}` - Get room details
- `GET /room/{id}/stats` - Get current room statistics
- `POST /room/{id}/join` - Join room (authenticated)
- `DELETE /room/{id}` - Delete room (host only)

**Bot Management**
- `POST /room/{id}/bot` - Add AI bot to room
- `DELETE /room/{id}/bot/{bot_uuid}` - Remove bot from room

### WebSocket Protocol

**Connection**
- Endpoint: `GET /ws/{room_id}`
- Authentication: JWT token via `Sec-WebSocket-Protocol` header

**Client → Server Messages**
- `CHAT` - Send chat message
- `MOVE` - Play cards
- `LEAVE` - Leave room
- `START_GAME` - Start game (host only)
- `READY` - Mark ready for game

**Server → Client Messages**
- `PLAYERS_LIST` - Current players in room
- `MOVE_PLAYED` - Player made a move
- `TURN_CHANGE` - Turn advanced to next player
- `GAME_STARTED` - Game has begun
- `GAME_WON` - Player won the game
- `GAME_RESET` - Game state reset
- `BOT_ADDED` / `BOT_REMOVED` - Bot status change
- `STATS_UPDATED` - Statistics updated
- `ERROR` - Error occurred
- `HOST_CHANGE` - New host assigned

## Game Rules

Big Two is a climbing card game where players try to be first to empty their hand.

**Card Rankings**
- Suits: Diamonds < Clubs < Hearts < Spades
- Ranks: 3 < 4 < 5 < 6 < 7 < 8 < 9 < 10 < J < Q < K < A < 2 (2 is highest)

**Valid Hands**
- Single card
- Pair (two cards of same rank)
- Triple (three cards of same rank)
- Straight (five consecutive ranks)
- Flush (five cards of same suit)
- Full House (triple + pair)
- Four of a Kind (four cards of same rank)
- Straight Flush (five consecutive cards of same suit)

**Special Rules**
- First move must include 3♦
- Players must play higher than previous hand or pass
- When all players pass, last player starts new round with any hand

## Development

### Running Tests
```bash
# Backend
cargo test                           # Unit tests
cargo test -- --ignored              # Integration tests
cargo test test_name -- --nocapture  # Single test with output

# Frontend
npm test
```

### Code Quality
Before committing, ensure:
1. `cargo test` - All tests pass
2. `cargo clippy` - No warnings (CI enforces `-D warnings`)
3. `cargo fmt` - Code is formatted

### Manual Testing
```bash
./scripts/test-session.sh     # Test REST endpoints
```

## Configuration

**Environment Variables**
- `DATABASE_URL` - PostgreSQL connection string (optional)
- `SESSION_EXPIRATION_DAYS` - Session lifetime in days (default: 365)
- `PORT` - Server port (default: 3000)
- `AI_BOT_SERVICE_URL` - Base URL for external bot inference service (required for remote AI inference)
- `AI_BOT_SERVICE_PREDICT_PATH` - Predict endpoint path appended to base URL (default: `/api/v1/predict`)
- `AI_BOT_HTTP_TIMEOUT_MS` - Timeout for AI bot HTTP requests in milliseconds (default: `1200`)

If `AI_BOT_SERVICE_URL` is unset, AI difficulty bots log a warning and fall back to `BasicBotStrategy`.

## License

MIT

## Credits

Built with Rust, axum, React, and TypeScript.
