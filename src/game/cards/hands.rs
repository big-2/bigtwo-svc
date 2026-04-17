use thiserror::Error;

use super::basic::{Card, Rank, Suit};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HandError {
    #[error("Invalid hand size")]
    InvalidHandSize,
    #[error("Invalid hand type")]
    InvalidHandType,
    #[error("Cards not sorted")]
    #[allow(dead_code)] // Error variant for validation
    CardsNotSorted,
    #[error("Cannot compare across types")]
    #[allow(dead_code)] // Error variant for validation
    CannotCompareAcrossTypes,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SingleHand {
    pub card: Card,
}

impl SingleHand {
    pub fn new(card: Card) -> Self {
        Self { card }
    }
}

impl PartialOrd for SingleHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SingleHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.card.cmp(&other.card)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PairHand {
    pub rank: Rank,
    pub cards: [Card; 2], // Store both cards in the pair
    pub high_card: Card,  // The higher suit of the pair (kept for comparison)
}

impl PairHand {
    pub fn new(card1: Card, card2: Card) -> Result<Self, HandError> {
        if card1.rank != card2.rank {
            return Err(HandError::InvalidHandType);
        }

        let high_card = if card1.suit > card2.suit {
            card1
        } else {
            card2
        };

        // Store cards in a consistent order: lower suit first, higher suit second
        let cards = if card1.suit < card2.suit {
            [card1, card2]
        } else {
            [card2, card1]
        };

        Ok(Self {
            rank: card1.rank,
            cards,
            high_card,
        })
    }
}

impl PartialOrd for PairHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PairHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.rank.cmp(&other.rank) {
            std::cmp::Ordering::Equal => self.high_card.suit.cmp(&other.high_card.suit),
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TripleHand {
    pub rank: Rank,
    pub cards: [Card; 3], // Store all three cards in the triple
    pub high_card: Card,  // The highest suit of the triple (kept for comparison)
}

impl TripleHand {
    pub fn new(card1: Card, card2: Card, card3: Card) -> Result<Self, HandError> {
        if !(card1.rank == card2.rank && card2.rank == card3.rank) {
            return Err(HandError::InvalidHandType);
        }

        let high_card = *[card1, card2, card3]
            .iter()
            .max()
            .expect("TripleHand should have exactly 3 cards for max calculation");

        // Store cards in sorted order by suit
        let mut cards = [card1, card2, card3];
        cards.sort_by_key(|card| card.suit);

        Ok(Self {
            rank: card1.rank,
            cards,
            high_card,
        })
    }
}

impl PartialOrd for TripleHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TripleHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.rank.cmp(&other.rank) {
            std::cmp::Ordering::Equal => self.high_card.suit.cmp(&other.high_card.suit),
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StraightHand {
    pub cards: Vec<Card>,
    pub high_card: Card,
}

impl StraightHand {
    pub fn new(cards: Vec<Card>) -> Self {
        assert!(!cards.is_empty(), "StraightHand requires at least one card");
        let high_card = FiveCardHand::straight_high_card(&cards)
            .expect("StraightHand should only be constructed from valid straights");
        Self { cards, high_card }
    }
}

impl PartialOrd for StraightHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StraightHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Straights compare by high card only
        self.high_card.cmp(&other.high_card)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlushHand {
    pub cards: Vec<Card>,
    pub suit: Suit,
    pub high_card: Card,
}

impl FlushHand {
    pub fn new(cards: Vec<Card>) -> Self {
        let suit = cards[0].suit; // All cards have same suit in a flush
        let high_card = *cards
            .iter()
            .max()
            .expect("FlushHand should have cards for max calculation");
        Self {
            cards,
            suit,
            high_card,
        }
    }
}

impl PartialOrd for FlushHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FlushHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Flushes: compare by suit first, then by highest card
        match self.suit.cmp(&other.suit) {
            std::cmp::Ordering::Equal => self.high_card.cmp(&other.high_card),
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FullHouseHand {
    pub cards: Vec<Card>,
    pub triple_rank: Rank,
    pub high_card: Card,
}

impl FullHouseHand {
    pub fn new(cards: Vec<Card>) -> Self {
        let rank_counts = Self::count_ranks(&cards);
        let triple_rank = *rank_counts
            .iter()
            .find(|(_, &count)| count == 3)
            .expect("FullHouseHand should have exactly one rank with 3 cards")
            .0;
        let high_card = *cards
            .iter()
            .max()
            .expect("FullHouseHand should have cards for max calculation");
        Self {
            cards,
            triple_rank,
            high_card,
        }
    }

    fn count_ranks(cards: &[Card]) -> std::collections::HashMap<Rank, usize> {
        let mut counts = std::collections::HashMap::new();
        for card in cards {
            *counts.entry(card.rank).or_insert(0) += 1;
        }
        counts
    }
}

impl PartialOrd for FullHouseHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FullHouseHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Full houses compare by the rank of the triple
        self.triple_rank.cmp(&other.triple_rank)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FourOfAKindHand {
    pub cards: Vec<Card>,
    pub quad_rank: Rank,
    pub high_card: Card,
}

impl FourOfAKindHand {
    pub fn new(cards: Vec<Card>) -> Self {
        let rank_counts = Self::count_ranks(&cards);
        let quad_rank = *rank_counts
            .iter()
            .find(|(_, &count)| count == 4)
            .expect("FourOfAKindHand should have exactly one rank with 4 cards")
            .0;
        let high_card = *cards
            .iter()
            .max()
            .expect("FourOfAKindHand should have cards for max calculation");
        Self {
            cards,
            quad_rank,
            high_card,
        }
    }

    fn count_ranks(cards: &[Card]) -> std::collections::HashMap<Rank, usize> {
        let mut counts = std::collections::HashMap::new();
        for card in cards {
            *counts.entry(card.rank).or_insert(0) += 1;
        }
        counts
    }
}

impl PartialOrd for FourOfAKindHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FourOfAKindHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Four of a kind compares by the rank of the quad
        self.quad_rank.cmp(&other.quad_rank)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StraightFlushHand {
    pub cards: Vec<Card>,
    pub suit: Suit,
    pub high_card: Card,
}

impl StraightFlushHand {
    pub fn new(cards: Vec<Card>) -> Self {
        let suit = cards[0].suit; // All cards have same suit
        let high_card = FiveCardHand::straight_high_card(&cards)
            .expect("StraightFlushHand should only be constructed from valid straights");
        Self {
            cards,
            suit,
            high_card,
        }
    }
}

impl PartialOrd for StraightFlushHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StraightFlushHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Straight flushes: compare by suit first, then by highest card
        match self.suit.cmp(&other.suit) {
            std::cmp::Ordering::Equal => self.high_card.cmp(&other.high_card),
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FiveCardHand {
    Straight(StraightHand),
    Flush(FlushHand),
    FullHouse(FullHouseHand),
    FourOfAKind(FourOfAKindHand),
    StraightFlush(StraightFlushHand),
}

impl FiveCardHand {
    pub fn new(cards: &[Card]) -> Result<Self, HandError> {
        if cards.len() != 5 {
            return Err(HandError::InvalidHandSize);
        }

        let mut cards = cards.to_owned();
        cards.sort();
        Self::classify_hand(cards)
    }

    fn classify_hand(cards: Vec<Card>) -> Result<Self, HandError> {
        let is_flush = cards.iter().all(|card| card.suit == cards[0].suit);
        let is_straight = Self::is_straight(&cards);

        if is_straight && is_flush {
            return Ok(FiveCardHand::StraightFlush(StraightFlushHand::new(cards)));
        }

        // Check for four of a kind
        let rank_counts = Self::count_ranks(&cards);
        if rank_counts.values().any(|&count| count == 4) {
            return Ok(FiveCardHand::FourOfAKind(FourOfAKindHand::new(cards)));
        }

        // Check for full house
        let mut counts: Vec<_> = rank_counts.values().collect();
        counts.sort();
        if counts == vec![&2, &3] {
            return Ok(FiveCardHand::FullHouse(FullHouseHand::new(cards)));
        }

        if is_flush {
            return Ok(FiveCardHand::Flush(FlushHand::new(cards)));
        }

        if is_straight {
            return Ok(FiveCardHand::Straight(StraightHand::new(cards)));
        }

        Err(HandError::InvalidHandType)
    }

    fn is_straight(cards: &[Card]) -> bool {
        // Extract and sort ranks
        let mut ranks: Vec<Rank> = cards.iter().map(|c| c.rank).collect();
        ranks.sort();

        // Special case 1: Ace-low straight (A-2-3-4-5)
        // Ace acts as value 1, Two acts as value 2
        if ranks == vec![Rank::Three, Rank::Four, Rank::Five, Rank::Ace, Rank::Two] {
            return true;
        }

        // Special case 2: Two-low straight (2-3-4-5-6)
        if ranks == vec![Rank::Three, Rank::Four, Rank::Five, Rank::Six, Rank::Two] {
            return true;
        }

        // Special case 3: Ace-high straight (10-J-Q-K-A)
        // Ace acts as value 14 (after King)
        if ranks == vec![Rank::Ten, Rank::Jack, Rank::Queen, Rank::King, Rank::Ace] {
            return true;
        }

        // Normal consecutive straights: 3-4-5-6-7 through 9-10-J-Q-K
        // These should be consecutive in rank value and not contain Ace or Two
        // (because Ace and Two have special positioning rules)
        if ranks.contains(&Rank::Ace) || ranks.contains(&Rank::Two) {
            return false; // Any other combination with Ace or Two is invalid
        }

        // Check if remaining ranks are consecutive
        for i in 1..ranks.len() {
            let prev_rank_value = ranks[i - 1] as u8;
            let curr_rank_value = ranks[i] as u8;

            if curr_rank_value != prev_rank_value + 1 {
                return false;
            }
        }

        true
    }

    fn straight_high_card(cards: &[Card]) -> Option<Card> {
        let mut ranks: Vec<Rank> = cards.iter().map(|c| c.rank).collect();
        ranks.sort();

        let high_rank = if ranks == vec![Rank::Three, Rank::Four, Rank::Five, Rank::Ace, Rank::Two]
        {
            Rank::Five
        } else if ranks == vec![Rank::Three, Rank::Four, Rank::Five, Rank::Six, Rank::Two] {
            Rank::Six
        } else if ranks == vec![Rank::Ten, Rank::Jack, Rank::Queen, Rank::King, Rank::Ace] {
            Rank::Ace
        } else if !Self::is_straight(cards) {
            return None;
        } else {
            *ranks
                .last()
                .expect("straight_high_card requires at least one card")
        };

        cards.iter().copied().find(|card| card.rank == high_rank)
    }

    fn count_ranks(cards: &[Card]) -> std::collections::HashMap<Rank, usize> {
        let mut counts = std::collections::HashMap::new();
        for card in cards {
            *counts.entry(card.rank).or_insert(0) += 1;
        }
        counts
    }

    pub fn hand_type_value(&self) -> u8 {
        match self {
            FiveCardHand::Straight(_) => 0,
            FiveCardHand::Flush(_) => 1,
            FiveCardHand::FullHouse(_) => 2,
            FiveCardHand::FourOfAKind(_) => 3,
            FiveCardHand::StraightFlush(_) => 4,
        }
    }
}

impl PartialOrd for FiveCardHand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FiveCardHand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.hand_type_value().cmp(&other.hand_type_value()) {
            std::cmp::Ordering::Equal => {
                // Same hand type - delegate to specific type comparison
                match (self, other) {
                    (FiveCardHand::Straight(a), FiveCardHand::Straight(b)) => a.cmp(b),
                    (FiveCardHand::Flush(a), FiveCardHand::Flush(b)) => a.cmp(b),
                    (FiveCardHand::FullHouse(a), FiveCardHand::FullHouse(b)) => a.cmp(b),
                    (FiveCardHand::FourOfAKind(a), FiveCardHand::FourOfAKind(b)) => a.cmp(b),
                    (FiveCardHand::StraightFlush(a), FiveCardHand::StraightFlush(b)) => a.cmp(b),
                    _ => unreachable!("Same hand type value but different variants"),
                }
            }
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Hand {
    Pass,
    Single(SingleHand),
    Pair(PairHand),
    Triple(TripleHand),
    Five(FiveCardHand),
}

impl Hand {
    /// Create a hand from a vector of cards
    pub fn from_cards(cards: &[Card]) -> Result<Self, HandError> {
        match cards.len() {
            0 => Ok(Hand::Pass),
            1 => Ok(Hand::Single(SingleHand::new(cards[0]))),
            2 => Ok(Hand::Pair(PairHand::new(cards[0], cards[1])?)),
            3 => Ok(Hand::Triple(TripleHand::new(cards[0], cards[1], cards[2])?)),
            5 => Ok(Hand::Five(FiveCardHand::new(cards)?)),
            _ => Err(HandError::InvalidHandSize),
        }
    }

    /// Check if this hand can beat another hand according to Big Two rules
    /// Only hands of the same type can be compared (except all 5-card combos can compete)
    pub fn can_beat(&self, other: &Hand) -> bool {
        match (self, other) {
            (Hand::Pass, _) => true,
            (_, Hand::Pass) => false,
            (Hand::Single(a), Hand::Single(b)) => a > b,
            (Hand::Pair(a), Hand::Pair(b)) => a > b,
            (Hand::Triple(a), Hand::Triple(b)) => a > b,
            (Hand::Five(a), Hand::Five(b)) => a > b,
            _ => false, // Cannot compare across different hand types
        }
    }

    /// Get the hand type name for display
    #[allow(dead_code)] // Public API for UI display
    pub fn hand_type_name(&self) -> &'static str {
        match self {
            Hand::Pass => "Pass",
            Hand::Single(_) => "Single",
            Hand::Pair(_) => "Pair",
            Hand::Triple(_) => "Triple",
            Hand::Five(five) => match five {
                FiveCardHand::Straight(_) => "Straight",
                FiveCardHand::Flush(_) => "Flush",
                FiveCardHand::FullHouse(_) => "Full House",
                FiveCardHand::FourOfAKind(_) => "Four of a Kind",
                FiveCardHand::StraightFlush(_) => "Straight Flush",
            },
        }
    }

    /// Get the cards that make up this hand
    pub fn to_cards(&self) -> Vec<Card> {
        match self {
            Hand::Pass => vec![],
            Hand::Single(single) => vec![single.card],
            Hand::Pair(pair) => {
                // Return all cards in the pair
                pair.cards.to_vec()
            }
            Hand::Triple(triple) => {
                // Return all cards in the triple
                triple.cards.to_vec()
            }
            Hand::Five(five) => match five {
                FiveCardHand::Straight(straight) => straight.cards.clone(),
                FiveCardHand::Flush(flush) => flush.cards.clone(),
                FiveCardHand::FullHouse(full_house) => full_house.cards.clone(),
                FiveCardHand::FourOfAKind(four_kind) => four_kind.cards.clone(),
                FiveCardHand::StraightFlush(straight_flush) => straight_flush.cards.clone(),
            },
        }
    }
}

/// Compare two sets of played cards to determine if the first can beat the second
/// Returns Ok(true) if played_cards can beat current_cards, Ok(false) otherwise
/// Empty played_cards represents a pass
#[allow(dead_code)] // Public API for future use
pub fn compare_played_cards(
    played_cards: &[Card],
    current_cards: &[Card],
) -> Result<bool, HandError> {
    let played_hand = if played_cards.is_empty() {
        Hand::Pass
    } else {
        Hand::from_cards(played_cards)?
    };

    let current_hand = if current_cards.is_empty() {
        Hand::Pass
    } else {
        Hand::from_cards(current_cards)?
    };

    Ok(played_hand.can_beat(&current_hand))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_single_hand() {
        let card1 = Card::new(Rank::King, Suit::Hearts);
        let card2 = Card::new(Rank::Queen, Suit::Spades);

        let hand1 = Hand::from_cards(&[card1]).unwrap();
        let hand2 = Hand::from_cards(&[card2]).unwrap();

        assert!(hand1.can_beat(&hand2));

        // Test that we can't compare across types
        let pair = Hand::from_cards(&[
            Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Three, Suit::Spades),
        ])
        .unwrap();

        assert!(!hand1.can_beat(&pair)); // Single cannot beat pair
        assert!(!pair.can_beat(&hand1)); // Pair cannot beat single
    }

    #[test]
    fn test_pair_hand() {
        let pair1 = Hand::from_cards(&[
            Card::new(Rank::King, Suit::Hearts),
            Card::new(Rank::King, Suit::Spades),
        ])
        .unwrap();

        let pair2 = Hand::from_cards(&[
            Card::new(Rank::Queen, Suit::Diamonds),
            Card::new(Rank::Queen, Suit::Clubs),
        ])
        .unwrap();

        assert!(pair1.can_beat(&pair2));

        // Test same rank pairs with different suits
        let pair3 = Hand::from_cards(&[
            Card::new(Rank::King, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
        ])
        .unwrap();

        assert!(pair1.can_beat(&pair3)); // Spades beats Clubs for the high card
    }

    #[test]
    fn test_triple_hand() {
        let triple1 = Hand::from_cards(&[
            Card::new(Rank::King, Suit::Hearts),
            Card::new(Rank::King, Suit::Spades),
            Card::new(Rank::King, Suit::Diamonds),
        ])
        .unwrap();

        let triple2 = Hand::from_cards(&[
            Card::new(Rank::Queen, Suit::Hearts),
            Card::new(Rank::Queen, Suit::Spades),
            Card::new(Rank::Queen, Suit::Diamonds),
        ])
        .unwrap();

        assert!(triple1.can_beat(&triple2));
    }

    #[test]
    fn test_construct_invalid_straight() {
        let straight = Hand::from_cards(&[
            Card::new(Rank::Jack, Suit::Hearts),
            Card::new(Rank::Queen, Suit::Spades),
            Card::new(Rank::King, Suit::Diamonds),
            Card::new(Rank::Ace, Suit::Clubs),
            Card::new(Rank::Two, Suit::Hearts),
        ]);

        assert!(straight.is_err());
        assert_eq!(straight.err(), Some(HandError::InvalidHandType));
    }

    #[test]
    fn test_construct_valid_straight_with_ace() {
        let straight = Hand::from_cards(&[
            Card::new(Rank::Ace, Suit::Hearts),
            Card::new(Rank::Two, Suit::Spades),
            Card::new(Rank::Three, Suit::Diamonds),
            Card::new(Rank::Four, Suit::Clubs),
            Card::new(Rank::Five, Suit::Hearts),
        ]);

        assert!(straight.is_ok());
    }

    #[test]
    fn test_construct_valid_straight_with_two() {
        let straight = Hand::from_cards(&[
            Card::new(Rank::Two, Suit::Hearts),
            Card::new(Rank::Three, Suit::Spades),
            Card::new(Rank::Four, Suit::Diamonds),
            Card::new(Rank::Five, Suit::Clubs),
            Card::new(Rank::Six, Suit::Hearts),
        ]);

        assert!(straight.is_ok());
    }

    #[test]
    fn test_construct_valid_ace_high_straight() {
        let straight = Hand::from_cards(&[
            Card::new(Rank::Ten, Suit::Hearts),
            Card::new(Rank::Jack, Suit::Spades),
            Card::new(Rank::Queen, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
            Card::new(Rank::Ace, Suit::Hearts),
        ]);

        assert!(straight.is_ok());
        match straight.unwrap() {
            Hand::Five(FiveCardHand::Straight(_)) => (),
            _ => panic!("Expected straight hand"),
        }
    }

    #[test]
    fn test_straight() {
        let straight = Hand::from_cards(&[
            Card::new(Rank::Six, Suit::Clubs),
            Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Four, Suit::Spades),
            Card::new(Rank::Seven, Suit::Hearts),
        ])
        .unwrap();

        match straight {
            Hand::Five(five) => match five {
                FiveCardHand::Straight(_) => (),
                _ => panic!("Expected straight hand"),
            },
            _ => panic!("Expected five card hand"),
        }
    }

    #[rstest]
    #[case(vec![
        Card::new(Rank::King, Suit::Hearts),
        Card::new(Rank::Ace, Suit::Spades),
        Card::new(Rank::Two, Suit::Diamonds),
        Card::new(Rank::Three, Suit::Clubs),
        Card::new(Rank::Four, Suit::Hearts),
    ])] // K-A-2-3-4 wraparound
    #[case(vec![
        Card::new(Rank::Queen, Suit::Hearts),
        Card::new(Rank::King, Suit::Spades),
        Card::new(Rank::Ace, Suit::Diamonds),
        Card::new(Rank::Two, Suit::Clubs),
        Card::new(Rank::Three, Suit::Hearts),
    ])] // Q-K-A-2-3 wraparound
    #[case(vec![
        Card::new(Rank::Ace, Suit::Hearts),
        Card::new(Rank::Two, Suit::Spades),
        Card::new(Rank::Three, Suit::Diamonds),
        Card::new(Rank::Four, Suit::Clubs),
        Card::new(Rank::Six, Suit::Hearts),
    ])] // A-2-3-4-6 (not consecutive)
    #[case(vec![
        Card::new(Rank::Two, Suit::Hearts),
        Card::new(Rank::Three, Suit::Spades),
        Card::new(Rank::Four, Suit::Diamonds),
        Card::new(Rank::Five, Suit::Clubs),
        Card::new(Rank::Seven, Suit::Hearts),
    ])] // 2-3-4-5-7 (not consecutive)
    fn test_invalid_straights(#[case] cards: Vec<Card>) {
        let result = Hand::from_cards(&cards);
        assert!(result.is_err());
        assert_eq!(result.err(), Some(HandError::InvalidHandType));
    }

    #[rstest]
    #[case(vec![
        Card::new(Rank::Three, Suit::Hearts),
        Card::new(Rank::Four, Suit::Spades),
        Card::new(Rank::Five, Suit::Diamonds),
        Card::new(Rank::Six, Suit::Clubs),
        Card::new(Rank::Seven, Suit::Hearts),
    ])] // 3-4-5-6-7
    #[case(vec![
        Card::new(Rank::Seven, Suit::Hearts),
        Card::new(Rank::Eight, Suit::Spades),
        Card::new(Rank::Nine, Suit::Diamonds),
        Card::new(Rank::Ten, Suit::Clubs),
        Card::new(Rank::Jack, Suit::Hearts),
    ])] // 7-8-9-10-J
    #[case(vec![
        Card::new(Rank::Nine, Suit::Hearts),
        Card::new(Rank::Ten, Suit::Spades),
        Card::new(Rank::Jack, Suit::Diamonds),
        Card::new(Rank::Queen, Suit::Clubs),
        Card::new(Rank::King, Suit::Hearts),
    ])] // 9-10-J-Q-K
    #[case(vec![
        Card::new(Rank::Two, Suit::Hearts),
        Card::new(Rank::Three, Suit::Spades),
        Card::new(Rank::Four, Suit::Diamonds),
        Card::new(Rank::Five, Suit::Clubs),
        Card::new(Rank::Six, Suit::Hearts),
    ])] // 2-3-4-5-6
    fn test_valid_normal_straights(#[case] cards: Vec<Card>) {
        let result = Hand::from_cards(&cards);
        assert!(result.is_ok());
        match result.unwrap() {
            Hand::Five(FiveCardHand::Straight(_)) => (),
            _ => panic!("Expected straight hand"),
        }
    }

    #[rstest]
    #[case(Hand::from_cards(&[Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Four, Suit::Spades),
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Six, Suit::Clubs),
            Card::new(Rank::Seven, Suit::Hearts)])
        .unwrap(), true)]
    #[case(Hand::from_cards(&[Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Six, Suit::Spades),
            Card::new(Rank::Seven, Suit::Diamonds),
            Card::new(Rank::Eight, Suit::Clubs),
            Card::new(Rank::Nine, Suit::Diamonds)])
        .unwrap(), true)]
    #[case(Hand::from_cards(&[Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Six, Suit::Spades),
            Card::new(Rank::Seven, Suit::Diamonds),
            Card::new(Rank::Eight, Suit::Clubs),
            Card::new(Rank::Nine, Suit::Spades)])
        .unwrap(), false)]
    fn test_straight_comparison_with_straight(#[case] other_hand: Hand, #[case] expected: bool) {
        let straight = Hand::from_cards(&[
            Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Six, Suit::Spades),
            Card::new(Rank::Seven, Suit::Diamonds),
            Card::new(Rank::Eight, Suit::Clubs),
            Card::new(Rank::Nine, Suit::Hearts),
        ])
        .unwrap();

        assert_eq!(straight.can_beat(&other_hand), expected);
    }

    #[test]
    fn test_two_low_straight_beats_ace_low_straight() {
        let ace_low = Hand::from_cards(&[
            Card::new(Rank::Ace, Suit::Hearts),
            Card::new(Rank::Two, Suit::Spades),
            Card::new(Rank::Three, Suit::Diamonds),
            Card::new(Rank::Four, Suit::Clubs),
            Card::new(Rank::Five, Suit::Hearts),
        ])
        .unwrap();

        let two_low = Hand::from_cards(&[
            Card::new(Rank::Two, Suit::Hearts),
            Card::new(Rank::Three, Suit::Spades),
            Card::new(Rank::Four, Suit::Diamonds),
            Card::new(Rank::Five, Suit::Clubs),
            Card::new(Rank::Six, Suit::Hearts),
        ])
        .unwrap();

        assert!(two_low.can_beat(&ace_low));
        assert!(!ace_low.can_beat(&two_low));
    }

    #[rstest]
    #[case(Hand::from_cards(&[Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Six, Suit::Diamonds),
            Card::new(Rank::Seven, Suit::Diamonds),
            Card::new(Rank::Eight, Suit::Diamonds),
            Card::new(Rank::King, Suit::Diamonds)])
        .unwrap(), true)]
    #[case(Hand::from_cards(&[Card::new(Rank::Five, Suit::Spades),
            Card::new(Rank::Six, Suit::Spades),
            Card::new(Rank::Seven, Suit::Spades),
            Card::new(Rank::Eight, Suit::Spades),
            Card::new(Rank::Jack, Suit::Spades)])
        .unwrap(), false)]
    #[case(Hand::from_cards(&[Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Six, Suit::Hearts),
            Card::new(Rank::Seven, Suit::Hearts),
            Card::new(Rank::Eight, Suit::Hearts),
            Card::new(Rank::King, Suit::Hearts)])
        .unwrap(), false)]
    fn test_flush_comparison_with_flush(#[case] other_hand: Hand, #[case] expected: bool) {
        let flush = Hand::from_cards(&[
            Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Six, Suit::Hearts),
            Card::new(Rank::Seven, Suit::Hearts),
            Card::new(Rank::Eight, Suit::Hearts),
            Card::new(Rank::Jack, Suit::Hearts),
        ])
        .unwrap();

        assert_eq!(flush.can_beat(&other_hand), expected);
    }

    #[rstest]
    #[case(Hand::from_cards(&[Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Four, Suit::Spades),
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Six, Suit::Clubs),
            Card::new(Rank::Seven, Suit::Hearts)])
        .unwrap(), true)]
    #[case(Hand::from_cards(&[Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Seven, Suit::Hearts),
            Card::new(Rank::Nine, Suit::Hearts),
            Card::new(Rank::Jack, Suit::Hearts)])
        .unwrap(), true)]
    #[case(Hand::from_cards(&[Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Three, Suit::Spades),
            Card::new(Rank::Three, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
            Card::new(Rank::King, Suit::Hearts)])
        .unwrap(), true)]
    #[case(Hand::from_cards(&[Card::new(Rank::Two, Suit::Hearts),
            Card::new(Rank::Two, Suit::Spades),
            Card::new(Rank::Two, Suit::Diamonds),
            Card::new(Rank::Two, Suit::Clubs),
            Card::new(Rank::King, Suit::Hearts)])
        .unwrap(), false)]
    #[case(Hand::from_cards(&[Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Four, Suit::Hearts),
            Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Six, Suit::Hearts),
            Card::new(Rank::Seven, Suit::Hearts)])
        .unwrap(), false)]
    fn test_four_of_a_kind_comparison(#[case] other_hand: Hand, #[case] expected: bool) {
        let four_of_a_kind = Hand::from_cards(&[
            Card::new(Rank::Ace, Suit::Hearts),
            Card::new(Rank::Ace, Suit::Spades),
            Card::new(Rank::Ace, Suit::Diamonds),
            Card::new(Rank::Ace, Suit::Clubs),
            Card::new(Rank::King, Suit::Hearts),
        ])
        .unwrap();

        assert_eq!(four_of_a_kind.can_beat(&other_hand), expected);
    }

    #[test]
    fn test_full_house() {
        let full_house = Hand::from_cards(&[
            Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Three, Suit::Spades),
            Card::new(Rank::Three, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
            Card::new(Rank::King, Suit::Hearts),
        ])
        .unwrap();

        match full_house {
            Hand::Five(five) => match five {
                FiveCardHand::FullHouse(_) => (),
                _ => panic!("Expected full house"),
            },
            _ => panic!("Expected five card hand"),
        }
    }

    #[rstest]
    #[case(Hand::from_cards(&[Card::new(Rank::Three, Suit::Hearts),
            Card::new(Rank::Three, Suit::Spades),
            Card::new(Rank::Three, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
            Card::new(Rank::King, Suit::Hearts)])
        .unwrap(), true)]
    #[case(Hand::from_cards(&[Card::new(Rank::Queen, Suit::Hearts),
            Card::new(Rank::Queen, Suit::Spades),
            Card::new(Rank::Queen, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
            Card::new(Rank::King, Suit::Hearts)])
        .unwrap(), false)]
    fn test_full_house_comparison(#[case] other_hand: Hand, #[case] expected: bool) {
        let full_house = Hand::from_cards(&[
            Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Five, Suit::Spades),
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
            Card::new(Rank::King, Suit::Diamonds),
        ])
        .unwrap();

        assert_eq!(full_house.can_beat(&other_hand), expected);
    }

    #[rstest]
    #[case(vec![], "Pass")]
    #[case(vec![Card::new(Rank::King, Suit::Hearts)], "Single")]
    #[case(vec![
        Card::new(Rank::King, Suit::Hearts),
        Card::new(Rank::King, Suit::Spades),
    ], "Pair")]
    #[case(vec![
        Card::new(Rank::King, Suit::Hearts),
        Card::new(Rank::King, Suit::Spades),
        Card::new(Rank::King, Suit::Diamonds),
    ], "Triple")]
    #[case(vec![
        Card::new(Rank::Three, Suit::Hearts),
        Card::new(Rank::Four, Suit::Spades),
        Card::new(Rank::Five, Suit::Diamonds),
        Card::new(Rank::Six, Suit::Clubs),
        Card::new(Rank::Seven, Suit::Hearts),
    ], "Straight")]
    #[case(vec![
        Card::new(Rank::Three, Suit::Hearts),
        Card::new(Rank::Five, Suit::Hearts),
        Card::new(Rank::Seven, Suit::Hearts),
        Card::new(Rank::Nine, Suit::Hearts),
        Card::new(Rank::Jack, Suit::Hearts),
    ], "Flush")]
    #[case(vec![
        Card::new(Rank::Three, Suit::Hearts),
        Card::new(Rank::Three, Suit::Spades),
        Card::new(Rank::Three, Suit::Diamonds),
        Card::new(Rank::King, Suit::Clubs),
        Card::new(Rank::King, Suit::Hearts),
    ], "Full House")]
    #[case(vec![
        Card::new(Rank::Three, Suit::Hearts),
        Card::new(Rank::Three, Suit::Spades),
        Card::new(Rank::Three, Suit::Diamonds),
        Card::new(Rank::Three, Suit::Clubs),
        Card::new(Rank::King, Suit::Hearts),
    ], "Four of a Kind")]
    #[case(vec![
        Card::new(Rank::Three, Suit::Hearts),
        Card::new(Rank::Four, Suit::Hearts),
        Card::new(Rank::Five, Suit::Hearts),
        Card::new(Rank::Six, Suit::Hearts),
        Card::new(Rank::Seven, Suit::Hearts),
    ], "Straight Flush")]
    fn test_hand_type_names(#[case] cards: Vec<Card>, #[case] expected_name: &str) {
        let hand = Hand::from_cards(&cards).unwrap();
        assert_eq!(hand.hand_type_name(), expected_name);
    }

    #[test]
    fn test_invalid_hands() {
        // Invalid pair (different ranks)
        let result = Hand::from_cards(&[
            Card::new(Rank::King, Suit::Hearts),
            Card::new(Rank::Queen, Suit::Spades),
        ]);
        assert!(result.is_err());

        // Invalid hand size
        let result = Hand::from_cards(&[
            Card::new(Rank::King, Suit::Hearts),
            Card::new(Rank::King, Suit::Spades),
            Card::new(Rank::King, Suit::Diamonds),
            Card::new(Rank::King, Suit::Clubs),
        ]);
        assert!(result.is_err());
    }

    #[rstest]
    #[case(vec![], vec![], true)] // For technicality, pass 'beats' pass
    #[case(vec![], vec![Card::from_string("3D").unwrap()], true)] // Pass doesn't beat 3D
    #[case(vec![Card::from_string("KH").unwrap()], vec![Card::from_string("QS").unwrap()], true)] // QS doesn't beat KH
    #[case(vec![Card::from_string("QS").unwrap()], vec![Card::from_string("KH").unwrap()], false)] // KH beats QS
    #[case(vec![Card::from_string("KH").unwrap()], vec![Card::from_string("KH").unwrap()], false)] // Same card doesn't beat itself
    #[case(vec![Card::from_string("KH").unwrap(), Card::from_string("KS").unwrap()], vec![Card::from_string("AH").unwrap(), Card::from_string("AS").unwrap()], false)] // AH-AS doesn't beat KH-KS
    fn test_compare_played_cards(
        #[case] played_cards: Vec<Card>,
        #[case] current_cards: Vec<Card>,
        #[case] expected: bool,
    ) {
        let result = compare_played_cards(&played_cards, &current_cards);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }
}
