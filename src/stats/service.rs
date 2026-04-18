use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex as AsyncMutex, RwLock};

use crate::{
    bot::BotManager,
    event::{RoomEvent, RoomEventError, RoomEventHandler},
    game::Game,
    room::service::RoomService,
};

use super::{
    calculators::{CardCountScoreCalculator, TenPlusMultiplierCalculator},
    collectors::{CardsRemainingCollector, WinLossCollector},
    repository::{GameHistoryRepository, StatsRepository},
    CalculationContext, CollectedData, GameResult, PlayerGameResult, RoomStats, ScoreCalculator,
    StatCollector, StatsError, StatsGameFilter,
};

pub struct StatsService {
    collectors: Vec<Arc<dyn StatCollector>>,
    calculators: Vec<Arc<dyn ScoreCalculator>>,
    repository: Arc<dyn StatsRepository>,
    game_history_repository: Option<Arc<dyn GameHistoryRepository>>,
    bot_manager: Option<Arc<BotManager>>,
    room_mutexes: Arc<RwLock<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

impl StatsService {
    pub fn builder(repository: Arc<dyn StatsRepository>) -> StatsServiceBuilder {
        StatsServiceBuilder::new(repository)
    }

    #[allow(dead_code)] // Public API for inspecting configured collectors
    pub fn collectors(&self) -> Vec<Arc<dyn StatCollector>> {
        self.collectors.clone()
    }

    pub async fn process_completed_game(
        &self,
        room_id: &str,
        game: &Game,
        winner_uuid: &str,
    ) -> Result<(GameResult, RoomStats), StatsError> {
        let room_lock = self.room_lock(room_id).await;
        let _guard = room_lock.lock().await;

        // Capture timestamp once at the start
        let completed_at = chrono::Utc::now();
        let game_number = self.next_game_number(room_id).await?;
        let had_bots = self.room_contains_bots(room_id).await;

        let collected = self.collect_all(game, winner_uuid).await?;

        let player_metadata: Vec<(String, usize)> = game
            .players()
            .iter()
            .map(|p| (p.uuid.clone(), p.cards.len()))
            .collect();

        let (raw_scores, final_scores) = self.calculate_scores(
            &player_metadata,
            &collected,
            room_id,
            game_number,
            winner_uuid,
            had_bots,
            completed_at,
        );

        let player_results =
            self.build_player_results(game, winner_uuid, &raw_scores, &final_scores);
        let moves = game
            .move_log()
            .iter()
            .map(|mv| super::GameMoveResult {
                sequence: mv.sequence,
                player_uuid: mv.player_uuid.clone(),
                action: mv.action,
                cards: mv.cards.clone(),
            })
            .collect();

        let game_result = GameResult {
            game_id: format!("{}:{}", game.id(), game.created_at().timestamp_millis()),
            room_id: room_id.to_string(),
            game_number,
            winner_uuid: winner_uuid.to_string(),
            players: player_results,
            moves,
            started_at: game.created_at(),
            completed_at,
            had_bots,
        };

        let inserted = if let Some(history_repository) = &self.game_history_repository {
            history_repository
                .record_completed_game(&game_result)
                .await?
        } else {
            true
        };

        if !inserted {
            let room_stats = self
                .repository
                .get_room_stats(room_id)
                .await?
                .unwrap_or_else(|| RoomStats {
                    room_id: room_id.to_string(),
                    ..RoomStats::default()
                });
            return Ok((game_result, room_stats));
        }

        // Record game and get updated stats in one operation
        let updated_room_stats = self.repository.record_game(game_result.clone()).await?;

        Ok((game_result, updated_room_stats))
    }

    pub async fn get_room_stats(&self, room_id: &str) -> Result<Option<RoomStats>, StatsError> {
        self.repository.get_room_stats(room_id).await
    }

    pub async fn get_player_profile_stats(
        &self,
        player_uuid: &str,
        display_name: &str,
        filter: StatsGameFilter,
    ) -> Result<Option<super::PlayerProfileStatsResponse>, StatsError> {
        match &self.game_history_repository {
            Some(repository) => {
                repository
                    .get_player_profile_stats(player_uuid, display_name, filter)
                    .await
            }
            None => Ok(None),
        }
    }

    pub async fn get_recent_games_for_player(
        &self,
        player_uuid: &str,
        limit: u32,
        before: Option<chrono::DateTime<chrono::Utc>>,
        filter: StatsGameFilter,
    ) -> Result<super::PlayerRecentGamesResponse, StatsError> {
        match &self.game_history_repository {
            Some(repository) => {
                repository
                    .get_recent_games_for_player(player_uuid, limit, before, filter)
                    .await
            }
            None => Ok(super::PlayerRecentGamesResponse {
                games: vec![],
                next_before: None,
            }),
        }
    }

    pub async fn get_completed_game_for_player(
        &self,
        player_uuid: &str,
        game_id: &str,
    ) -> Result<Option<super::CompletedGameDetailResponse>, StatsError> {
        match &self.game_history_repository {
            Some(repository) => {
                repository
                    .get_completed_game_for_player(player_uuid, game_id)
                    .await
            }
            None => Ok(None),
        }
    }

    pub async fn reset_room_stats(&self, room_id: &str) -> Result<(), StatsError> {
        self.repository.reset_room_stats(room_id).await?;
        self.clear_room_lock(room_id).await;
        Ok(())
    }

    /// Calculate scores for all players using registered calculators
    #[allow(clippy::too_many_arguments)]
    fn calculate_scores(
        &self,
        player_metadata: &[(String, usize)],
        collected: &[CollectedData],
        room_id: &str,
        game_number: u32,
        winner_uuid: &str,
        had_bots: bool,
        completed_at: chrono::DateTime<chrono::Utc>,
    ) -> (HashMap<String, i32>, HashMap<String, i32>) {
        let mut current_scores: HashMap<String, i32> = HashMap::new();
        let mut raw_scores: HashMap<String, i32> = HashMap::new();

        for (index, calculator) in self.calculators.iter().enumerate() {
            let snapshot_players: Vec<PlayerGameResult> = player_metadata
                .iter()
                .map(|(uuid, cards)| PlayerGameResult {
                    uuid: uuid.clone(),
                    display_name: uuid.clone(),
                    won: uuid == winner_uuid,
                    cards_remaining: *cards as u8,
                    raw_score: current_scores.get(uuid).copied().unwrap_or_default(),
                    final_score: current_scores.get(uuid).copied().unwrap_or_default(),
                    turns_taken: 0,
                    passes: 0,
                    plays: 0,
                    cards_played: 0,
                    started_first: false,
                })
                .collect();

            let snapshot = GameResult {
                game_id: format!("{room_id}:{game_number}"),
                room_id: room_id.to_string(),
                game_number,
                winner_uuid: winner_uuid.to_string(),
                players: snapshot_players,
                moves: vec![],
                started_at: completed_at,
                completed_at,
                had_bots,
            };

            let context = CalculationContext::new(&snapshot, &current_scores);

            let mut next_scores = current_scores.clone();
            for (uuid, _) in player_metadata {
                let updated = calculator.calculate(uuid, collected, &context);
                next_scores.insert(uuid.clone(), updated);
            }

            if index == 0 {
                raw_scores = next_scores.clone();
            }

            current_scores = next_scores;
        }

        (raw_scores, current_scores)
    }

    async fn collect_all(
        &self,
        game: &Game,
        winner_uuid: &str,
    ) -> Result<Vec<CollectedData>, StatsError> {
        let mut collected = Vec::new();
        for collector in &self.collectors {
            collected.extend(collector.collect(game, winner_uuid).await?);
        }
        Ok(collected)
    }

    async fn room_lock(&self, room_id: &str) -> Arc<AsyncMutex<()>> {
        // Fast path: check if lock exists with read lock
        {
            let guard = self.room_mutexes.read().await;
            if let Some(lock) = guard.get(room_id) {
                return lock.clone();
            }
        }

        // Slow path: acquire write lock and create if needed
        // Note: or_insert_with is atomic - it only creates if the entry doesn't exist,
        // preventing duplicate mutex creation even if multiple threads reach here
        let mut guard = self.room_mutexes.write().await;
        guard
            .entry(room_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    async fn clear_room_lock(&self, room_id: &str) {
        let mut guard = self.room_mutexes.write().await;
        guard.remove(room_id);
    }

    async fn next_game_number(&self, room_id: &str) -> Result<u32, StatsError> {
        let existing = self.repository.get_room_stats(room_id).await?;
        Ok(existing.map(|stats| stats.games_played + 1).unwrap_or(1))
    }

    async fn room_contains_bots(&self, room_id: &str) -> bool {
        if let Some(bot_manager) = &self.bot_manager {
            let bots = bot_manager.get_bots_in_room(room_id).await;
            !bots.is_empty()
        } else {
            false
        }
    }

    fn build_player_results(
        &self,
        game: &Game,
        winner_uuid: &str,
        raw_scores: &HashMap<String, i32>,
        final_scores: &HashMap<String, i32>,
    ) -> Vec<PlayerGameResult> {
        game.players()
            .iter()
            .enumerate()
            .map(|(index, player)| {
                let mut turns_taken = 0_u32;
                let mut passes = 0_u32;
                let mut plays = 0_u32;
                let mut cards_played = 0_u32;

                for mv in game
                    .move_log()
                    .iter()
                    .filter(|mv| mv.player_uuid == player.uuid)
                {
                    turns_taken += 1;
                    match mv.action {
                        crate::game::MoveAction::Pass => passes += 1,
                        crate::game::MoveAction::Play => {
                            plays += 1;
                            cards_played += mv.cards.len() as u32;
                        }
                    }
                }

                PlayerGameResult {
                    uuid: player.uuid.clone(),
                    display_name: player.name.clone(),
                    won: player.uuid == winner_uuid,
                    cards_remaining: player.cards.len() as u8,
                    raw_score: raw_scores.get(&player.uuid).copied().unwrap_or_default(),
                    final_score: final_scores.get(&player.uuid).copied().unwrap_or_default(),
                    turns_taken,
                    passes,
                    plays,
                    cards_played,
                    started_first: index == 0,
                }
            })
            .collect()
    }
}

pub struct StatsServiceBuilder {
    collectors: Vec<Arc<dyn StatCollector>>,
    calculators: Vec<Arc<dyn ScoreCalculator>>,
    repository: Arc<dyn StatsRepository>,
    game_history_repository: Option<Arc<dyn GameHistoryRepository>>,
    bot_manager: Option<Arc<BotManager>>,
}

impl StatsServiceBuilder {
    fn new(repository: Arc<dyn StatsRepository>) -> Self {
        Self {
            collectors: vec![
                Arc::new(CardsRemainingCollector::new()),
                Arc::new(WinLossCollector::new()),
            ],
            calculators: vec![
                Arc::new(CardCountScoreCalculator::new()),
                Arc::new(TenPlusMultiplierCalculator::new()),
            ],
            repository,
            game_history_repository: None,
            bot_manager: None,
        }
    }

    #[allow(dead_code)] // Builder method for adding collectors
    pub fn with_collector(mut self, collector: Arc<dyn StatCollector>) -> Self {
        self.collectors.push(collector);
        self
    }

    #[allow(dead_code)] // Builder method for adding calculators
    pub fn with_calculator(mut self, calculator: Arc<dyn ScoreCalculator>) -> Self {
        self.calculators.push(calculator);
        self
    }

    pub fn with_bot_manager(mut self, bot_manager: Arc<BotManager>) -> Self {
        self.bot_manager = Some(bot_manager);
        self
    }

    pub fn with_game_history_repository(
        mut self,
        game_history_repository: Arc<dyn GameHistoryRepository>,
    ) -> Self {
        self.game_history_repository = Some(game_history_repository);
        self
    }

    pub fn build(mut self) -> StatsService {
        self.calculators.sort_by_key(|c| c.priority());
        StatsService {
            collectors: self.collectors,
            calculators: self.calculators,
            repository: self.repository,
            game_history_repository: self.game_history_repository,
            bot_manager: self.bot_manager,
            room_mutexes: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

pub struct StatsRoomSubscriber {
    stats_service: Arc<StatsService>,
    room_service: Arc<RoomService>,
    event_bus: crate::event::EventBus,
}

impl StatsRoomSubscriber {
    pub fn new(
        stats_service: Arc<StatsService>,
        room_service: Arc<RoomService>,
        event_bus: crate::event::EventBus,
    ) -> Self {
        Self {
            stats_service,
            room_service,
            event_bus,
        }
    }
}

#[async_trait::async_trait]
impl RoomEventHandler for StatsRoomSubscriber {
    async fn handle_room_event(
        &self,
        room_id: &str,
        event: RoomEvent,
    ) -> Result<(), RoomEventError> {
        match event {
            RoomEvent::GameWon { winner, game, .. } => {
                let result = self
                    .stats_service
                    .process_completed_game(room_id, &game, &winner)
                    .await;
                match result {
                    Ok((_game_result, room_stats)) => {
                        // Emit StatsUpdated event so WebSocket subscribers can broadcast
                        self.event_bus
                            .emit_to_room(room_id, RoomEvent::StatsUpdated { room_stats })
                            .await;
                    }
                    Err(err) => {
                        tracing::error!(
                            ?err,
                            room_id,
                            "Failed to process game completion for stats"
                        );
                    }
                }
            }
            RoomEvent::PlayerLeft { .. } => match self.room_service.get_room(room_id).await {
                Ok(Some(room)) if room.player_uuids.is_empty() => {
                    if let Err(err) = self.stats_service.reset_room_stats(room_id).await {
                        tracing::error!(?err, room_id, "Failed to reset stats after room emptied");
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::error!(?err, room_id, "Failed to load room for stats reset");
                }
            },
            _ => {}
        }

        Ok(())
    }

    fn handler_name(&self) -> &'static str {
        "StatsRoomSubscriber"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        bot::BotManager,
        game::{Card, Game},
        stats::{InMemoryGameHistoryRepository, InMemoryStatsRepository},
    };
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestCollector {
        calls: Arc<Mutex<u32>>,
    }

    #[async_trait::async_trait]
    impl StatCollector for TestCollector {
        async fn collect(
            &self,
            _game: &Game,
            winner_uuid: &str,
        ) -> Result<Vec<CollectedData>, StatsError> {
            let mut guard = self.calls.lock().await;
            *guard += 1;
            Ok(vec![CollectedData::WinLoss {
                player_uuid: winner_uuid.to_string(),
                won: true,
            }])
        }
    }

    struct BonusScoreCalculator;

    impl ScoreCalculator for BonusScoreCalculator {
        fn calculate(
            &self,
            player_uuid: &str,
            _collected_data: &[CollectedData],
            context: &CalculationContext,
        ) -> i32 {
            let base = context
                .current_scores
                .get(player_uuid)
                .copied()
                .unwrap_or_default();
            base + 4
        }

        fn priority(&self) -> u32 {
            300
        }
    }

    fn card(code: &str) -> Card {
        Card::from_string(code).unwrap()
    }

    fn game_with_players(players: Vec<(String, String, Vec<Card>)>) -> Game {
        Game::new_game_with_cards("room".to_string(), players).unwrap()
    }

    #[tokio::test]
    async fn process_completed_game_returns_game_and_stats() {
        let repo = Arc::new(InMemoryStatsRepository::new());
        let service = StatsService::builder(repo.clone()).build();

        let game = game_with_players(vec![
            (
                "Alice".to_string(),
                "alice".to_string(),
                (1..=13).map(|_| card("3D")).collect(),
            ),
            (
                "Bob".to_string(),
                "bob".to_string(),
                vec![card("4H"), card("5S")],
            ),
        ]);

        let (game_result, room_stats) = service
            .process_completed_game("room", &game, "bob")
            .await
            .unwrap();

        assert_eq!(game_result.room_id, "room");
        assert_eq!(game_result.game_number, 1);
        assert!(room_stats.player_stats.get("bob").unwrap().wins > 0);
        assert_eq!(room_stats.games_played, 1);

        let alice = game_result
            .players
            .iter()
            .find(|p| p.uuid == "alice")
            .unwrap();
        assert!(alice.raw_score > 0);
        assert!(alice.final_score >= alice.raw_score);
    }

    #[tokio::test]
    async fn process_completed_game_captures_moves_and_player_behavior_metrics() {
        let repo = Arc::new(InMemoryStatsRepository::new());
        let service = StatsService::builder(repo).build();

        let mut game = game_with_players(vec![
            (
                "Alice".to_string(),
                "alice".to_string(),
                vec![card("3D"), card("4D")],
            ),
            (
                "Bob".to_string(),
                "bob".to_string(),
                vec![card("5H"), card("6H")],
            ),
        ]);

        game.play_cards("alice", &[card("3D")]).unwrap();
        game.play_cards("bob", &[]).unwrap();
        game.play_cards("alice", &[card("4D")]).unwrap();

        let (game_result, _) = service
            .process_completed_game("room", &game, "alice")
            .await
            .unwrap();

        assert_eq!(game_result.moves.len(), 3);
        let alice = game_result
            .players
            .iter()
            .find(|player| player.uuid == "alice")
            .unwrap();
        assert!(alice.won);
        assert_eq!(alice.turns_taken, 2);
        assert_eq!(alice.passes, 0);
        assert_eq!(alice.plays, 2);
        assert_eq!(alice.cards_played, 2);
        assert!(alice.started_first);

        let bob = game_result
            .players
            .iter()
            .find(|player| player.uuid == "bob")
            .unwrap();
        assert!(!bob.won);
        assert_eq!(bob.turns_taken, 1);
        assert_eq!(bob.passes, 1);
        assert_eq!(bob.plays, 0);
        assert_eq!(bob.cards_played, 0);
    }

    #[tokio::test]
    async fn duplicate_completed_game_does_not_double_count_room_stats() {
        let repo = Arc::new(InMemoryStatsRepository::new());
        let history_repo = Arc::new(InMemoryGameHistoryRepository::new());
        let service = StatsService::builder(repo.clone())
            .with_game_history_repository(history_repo)
            .build();

        let game = game_with_players(vec![
            ("Alice".to_string(), "alice".to_string(), vec![card("3D")]),
            ("Bob".to_string(), "bob".to_string(), vec![card("4H")]),
        ]);

        let (_, room_stats) = service
            .process_completed_game("room", &game, "alice")
            .await
            .unwrap();
        assert_eq!(room_stats.games_played, 1);

        let (_, room_stats) = service
            .process_completed_game("room", &game, "alice")
            .await
            .unwrap();
        assert_eq!(room_stats.games_played, 1);
    }

    #[tokio::test]
    async fn calculates_game_numbers_sequentially_per_room() {
        let repo = Arc::new(InMemoryStatsRepository::new());
        let service = StatsService::builder(repo.clone()).build();

        let game = game_with_players(vec![
            ("Alice".to_string(), "alice".to_string(), vec![card("3D")]),
            ("Bob".to_string(), "bob".to_string(), vec![card("4H")]),
        ]);

        let (first_game, _) = service
            .process_completed_game("room", &game, "alice")
            .await
            .unwrap();
        assert_eq!(first_game.game_number, 1);

        let (second_game, _) = service
            .process_completed_game("room", &game, "bob")
            .await
            .unwrap();
        assert_eq!(second_game.game_number, 2);
    }

    #[tokio::test]
    async fn detects_bot_participation() {
        let repo = Arc::new(InMemoryStatsRepository::new());
        let bot_manager = Arc::new(BotManager::new());
        let bot_player = bot_manager
            .create_bot("room".to_string(), crate::bot::types::BotDifficulty::Easy)
            .await
            .unwrap();

        let service = StatsService::builder(repo.clone())
            .with_bot_manager(bot_manager.clone())
            .build();

        let game = game_with_players(vec![
            (
                bot_player.name.clone(),
                bot_player.uuid.clone(),
                vec![card("3D")],
            ),
            (
                "Human A".to_string(),
                "human-a".to_string(),
                vec![card("4C")],
            ),
            (
                "Human B".to_string(),
                "human-b".to_string(),
                vec![card("5H")],
            ),
            (
                "Human C".to_string(),
                "human-c".to_string(),
                vec![card("6S")],
            ),
        ]);

        let (game_result, _) = service
            .process_completed_game("room", &game, &bot_player.uuid)
            .await
            .unwrap();

        assert!(game_result.had_bots);
    }

    #[tokio::test]
    async fn honors_custom_collectors_and_calculators() {
        let repo = Arc::new(InMemoryStatsRepository::new());
        let test_collector = Arc::new(TestCollector::default());
        let service = StatsService::builder(repo.clone())
            .with_collector(test_collector.clone())
            .with_calculator(Arc::new(BonusScoreCalculator))
            .build();

        let game = game_with_players(vec![(
            "Alice".to_string(),
            "alice".to_string(),
            vec![card("3D")],
        )]);

        let (game_result, _) = service
            .process_completed_game("room", &game, "alice")
            .await
            .unwrap();

        assert_eq!(game_result.players[0].raw_score, 1);
        assert_eq!(game_result.players[0].final_score, 5);

        let call_count = *test_collector.calls.lock().await;
        assert_eq!(call_count, 1);
    }
}
