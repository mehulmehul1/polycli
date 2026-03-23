//! Temporal Arbitrage for Bitcoin Up/Down Markets
//!
//! Exploits the information flow from shorter-timeframe markets (5min)
//! to longer-timeframe markets (15min, 1hour, etc.)
//!
//! Key insight: A 15-minute market consists of 3 consecutive 5-minute markets.
//! We can observe the first two 5-minute results BEFORE the 15-minute market closes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A temporal chain of linked markets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalChain {
    /// The longer-timeframe market (15min, 1hour, etc.)
    pub target_market: MarketInfo,

    /// The constituent 5-minute markets
    pub five_min_markets: Vec<MarketInfo>,

    /// Reference price at chain start
    pub reference_price: f64,

    /// Chain type
    pub chain_type: ChainType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChainType {
    /// 3 x 5min = 15min
    FiveToFifteen,
    /// 4 x 15min = 1hour (12 x 5min)
    FifteenToOneHour,
    /// 4 x 1hour = 4hour (48 x 5min)
    OneHourToFourHour,
}

/// Market information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketInfo {
    pub condition_id: String,
    pub question: String,
    pub start_time: i64,
    pub end_time: i64,
    pub reference_price: f64,
    pub current_price: Option<f64>,
    pub yes_price: Option<f64>,
    pub no_price: Option<f64>,
}

/// Price move result
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PriceMove {
    Up { change: f64 },
    Down { change: f64 },
    Flat,
}

impl PriceMove {
    pub fn is_up(&self) -> bool {
        matches!(self, PriceMove::Up { .. })
    }

    pub fn change(&self) -> f64 {
        match self {
            PriceMove::Up { change } => *change,
            PriceMove::Down { change } => *change,
            PriceMove::Flat => 0.0,
        }
    }
}

/// Current phase in the temporal chain
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketPhase {
    /// No 5min markets resolved yet
    Initial,
    /// First 5min resolved (for 15min chain)
    AfterFirst,
    /// Second 5min resolved (for 15min chain)
    AfterSecond,
    /// All markets resolved
    Complete,
}

/// Transition probability matrix for Markov model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionMatrix {
    /// P(UP | previous UP)
    pub up_given_up: f64,
    /// P(DOWN | previous UP)
    pub down_given_up: f64,
    /// P(UP | previous DOWN)
    pub up_given_down: f64,
    /// P(DOWN | previous DOWN)
    pub down_given_down: f64,
}

impl TransitionMatrix {
    /// Create from historical data
    pub fn from_counts(
        up_after_up: usize,
        down_after_up: usize,
        up_after_down: usize,
        down_after_down: usize,
    ) -> Self {
        let total_after_up = up_after_up + down_after_up;
        let total_after_down = up_after_down + down_after_down;

        Self {
            up_given_up: (up_after_up as f64) / (total_after_up as f64).max(1.0),
            down_given_up: (down_after_up as f64) / (total_after_up as f64).max(1.0),
            up_given_down: (up_after_down as f64) / (total_after_down as f64).max(1.0),
            down_given_down: (down_after_down as f64) / (total_after_down as f64).max(1.0),
        }
    }

    /// Default from efficient market (no momentum)
    pub fn efficient_market() -> Self {
        Self {
            up_given_up: 0.5,
            down_given_up: 0.5,
            up_given_down: 0.5,
            down_given_down: 0.5,
        }
    }

    /// With slight momentum (typical for trending markets)
    pub fn slight_momentum() -> Self {
        Self {
            up_given_up: 0.52,
            down_given_up: 0.48,
            up_given_down: 0.48,
            down_given_down: 0.52,
        }
    }
}

/// Probability engine for temporal arbitrage
#[derive(Debug, Clone)]
pub struct TemporalProbabilityEngine {
    /// Transition probabilities
    pub transitions: TransitionMatrix,

    /// Volatility regime (0-1, higher = more volatile)
    pub volatility_regime: f64,

    /// Mean reversion strength (0-1)
    pub mean_reversion: f64,

    /// Historical cache for learning
    historical_cache: HashMap<String, Vec<PriceMove>>,
}

impl TemporalProbabilityEngine {
    pub fn new(transitions: TransitionMatrix) -> Self {
        Self {
            transitions,
            volatility_regime: 0.5,
            mean_reversion: 0.1,
            historical_cache: HashMap::new(),
        }
    }

    /// Calculate initial fair probability for 15min market
    ///
    /// Uses binomial tree with transition probabilities
    pub fn initial_fifteen_prob(&self) -> f64 {
        let p = self.transitions.up_given_up;  // P(UP | start, assume neutral)

        // 8 possible paths, count those with 2+ UPs:
        // U-U-U, U-U-D, U-D-U, D-U-U
        let p_up = p;
        let p_down = 1.0 - p;

        // Path probabilities:
        let p_uuu = p_up * p_up * p_up;
        let p_uud = p_up * p_up * p_down;
        let p_udu = p_up * p_down * p_up;
        let p_duu = p_down * p_up * p_up;

        p_uuu + p_uud + p_udu + p_duu
    }

    /// Calculate conditional probability after observing first 5min result
    pub fn conditional_after_first(
        &self,
        first_move: PriceMove,
        first_magnitude: f64,
    ) -> f64 {
        // After first move, we have 2 periods remaining
        // Update transition probabilities based on observed move

        let (p_up_next, p_down_next) = match first_move {
            PriceMove::Up { .. } => (
                self.transitions.up_given_up,
                self.transitions.down_given_up,
            ),
            PriceMove::Down { .. } => (
                self.transitions.up_given_down,
                self.transitions.down_given_down,
            ),
            PriceMove::Flat => (0.5, 0.5),
        };

        // Adjust for mean reversion based on magnitude
        let reversal_factor = if first_magnitude > 50.0 {
            // Large move: some mean reversion expected
            0.1 * self.mean_reversion
        } else {
            0.0
        };

        let p_up_adjusted = match first_move {
            PriceMove::Up { .. } => p_up_next - reversal_factor,
            PriceMove::Down { .. } => p_up_next + reversal_factor,
            PriceMove::Flat => p_up_next,
        };

        // For 15min to be UP, we need:
        // - UP-UP, UP-DOWN (started up), DOWN-UP, DOWN-DOWN (started down)... wait
        // Actually: final price > reference price
        //
        // After first UP at +Δ:
        // - UU: +Δ + something = UP
        // - UD: +Δ - something = depends on sizes
        //
        // Simplified: use the probability that final > current + something

        // Paths to UP after first move UP:
        // - UP-UP (prob: p_up_adjusted * p_up_adjusted)
        // - UP-DOWN where second move < first move (prob: p_up_adjusted * p_down_next * large_move_prob)
        //
        // For now, approximate:
        let p_two_more_ups = p_up_adjusted * self.transitions.up_given_up;
        let p_up_then_down = p_up_adjusted * self.transitions.down_given_up * 0.5;  // 50% chance down < up

        p_two_more_ups + p_up_then_down + 0.3  // Base probability even with mixed path
    }

    /// Calculate conditional probability after observing first two 5min results
    pub fn conditional_after_second(
        &self,
        first_move: PriceMove,
        second_move: PriceMove,
        first_magnitude: f64,
        second_magnitude: f64,
    ) -> f64 {
        // After two moves, one period remaining
        // 15min outcome heavily depends on cumulative move

        let cumulative = match (first_move, second_move) {
            (PriceMove::Up { change: a }, PriceMove::Up { change: b }) => a + b,
            (PriceMove::Up { change: a }, PriceMove::Down { change: b }) => a - b,
            (PriceMove::Down { change: a }, PriceMove::Up { change: b }) => b - a,
            (PriceMove::Down { change: a }, PriceMove::Down { change: b }) => -(a + b),
            (PriceMove::Flat, _) => second_magnitude,
            (_, PriceMove::Flat) => first_magnitude,
            (PriceMove::Flat, PriceMove::Flat) => 0.0,
        };

        // Base probability from transition
        let base_p = match second_move {
            PriceMove::Up { .. } => self.transitions.up_given_up,
            PriceMove::Down { .. } => self.transitions.up_given_down,
            PriceMove::Flat => 0.5,
        };

        // Adjust based on cumulative position
        if cumulative > 20.0 {
            // Strong UP position, high probability of staying UP
            (base_p + 0.7).min(0.95)
        } else if cumulative < -20.0 {
            // Strong DOWN position
            (base_p - 0.3).max(0.05)
        } else {
            // Near reference, final period decides
            base_p
        }
    }

    /// Calculate probability for 1hour market (four 15min periods)
    pub fn one_hour_prob(&self, observed_15min: &[PriceMove]) -> f64 {
        // 1hour = 4 x 15min = 12 x 5min
        // This is more complex - use binomial approximation

        match observed_15min.len() {
            0 => {
                // No info yet: use prior
                let p = self.transitions.up_given_up;
                // Approximate with 4-period binomial
                // P(3+ or 4 UPs) + P(2 UPs with large edge)
                // For now, return 0.5 for efficient market
                0.5
            }
            1 => {
                // One 15min done, 3 to go
                let first_was_up = observed_15min[0].is_up();
                let p = if first_was_up {
                    self.transitions.up_given_up
                } else {
                    self.transitions.up_given_down
                };
                p
            }
            n => {
                // n periods done, 4-n remaining
                // Count UPs so far
                let ups = observed_15min.iter().filter(|m| m.is_up()).count();
                // Simple heuristic
                match ups {
                    0 => 0.15,  // All DOWN so far
                    1 => 0.35,
                    2 => 0.5,
                    3 => 0.75,
                    _ => 0.9,   // All UP so far
                }
            }
        }
    }
}

/// Trading signal for temporal arbitrage
#[derive(Debug, Clone)]
pub struct TemporalSignal {
    pub chain_id: String,
    pub phase: MarketPhase,
    pub market_prob: f64,      // Current market price
    pub fair_prob: f64,        // Our calculated probability
    pub edge: f64,             // fair_prob - market_prob
    pub confidence: f64,       // 0-1
    pub action: SignalAction,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalAction {
    BuyYes,
    BuyNo,
    Hold,
    Reduce,
    Hedge,
}

/// Generate trading signals from temporal chains
pub fn generate_signals(
    chains: &[TemporalChain],
    engine: &TemporalProbabilityEngine,
    min_edge: f64,
) -> Vec<TemporalSignal> {
    chains.iter().map(|chain| {
        let phase = determine_phase(&chain.five_min_markets);

        let (market_prob, fair_prob) = match phase {
            MarketPhase::Initial => {
                let mp = chain.target_market.yes_price.unwrap_or(0.5);
                let fp = engine.initial_fifteen_prob();
                (mp, fp)
            }
            MarketPhase::AfterFirst => {
                if let Some(m1) = &chain.five_min_markets.get(0) {
                    if let (Some(final_price), Some(ref_price)) = (m1.current_price, Some(m1.reference_price)) {
                        let move1 = if final_price > ref_price {
                            PriceMove::Up { change: final_price - ref_price }
                        } else {
                            PriceMove::Down { change: ref_price - final_price }
                        };
                        let mp = chain.target_market.yes_price.unwrap_or(0.5);
                        let fp = engine.conditional_after_first(move1, move1.change());
                        (mp, fp)
                    } else {
                        (0.5, 0.5)
                    }
                } else {
                    (0.5, 0.5)
                }
            }
            MarketPhase::AfterSecond => {
                if let (Some(m1), Some(m2)) = (&chain.five_min_markets.get(0), &chain.five_min_markets.get(1)) {
                    if let (Some(f1), Some(r1), Some(f2), Some(r2)) = (
                        m1.current_price, Some(m1.reference_price),
                        m2.current_price, Some(m2.reference_price)
                    ) {
                        let move1 = if f1 > r1 {
                            PriceMove::Up { change: f1 - r1 }
                        } else {
                            PriceMove::Down { change: r1 - f1 }
                        };
                        let move2 = if f2 > r2 {
                            PriceMove::Up { change: f2 - r2 }
                        } else {
                            PriceMove::Down { change: r2 - f2 }
                        };
                        let mp = chain.target_market.yes_price.unwrap_or(0.5);
                        let fp = engine.conditional_after_second(move1, move2, move1.change(), move2.change());
                        (mp, fp)
                    } else {
                        (0.5, 0.5)
                    }
                } else {
                    (0.5, 0.5)
                }
            }
            MarketPhase::Complete => {
                (0.5, 0.5)
            }
        };

        let edge = fair_prob - market_prob;
        let confidence = edge.abs().min(1.0);

        let action = if edge > min_edge {
            SignalAction::BuyYes
        } else if edge < -min_edge {
            SignalAction::BuyNo
        } else {
            SignalAction::Hold
        };

        let reason = format!(
            "Phase: {:?}, Market: {:.3}, Fair: {:.3}, Edge: {:.3}",
            phase, market_prob, fair_prob, edge
        );

        TemporalSignal {
            chain_id: chain.target_market.condition_id.clone(),
            phase,
            market_prob,
            fair_prob,
            edge,
            confidence,
            action,
            reason,
        }
    }).filter(|s| s.action != SignalAction::Hold).collect()
}

/// Determine current phase of the temporal chain
fn determine_phase(five_min_markets: &[MarketInfo]) -> MarketPhase {
    let now = chrono::Utc::now().timestamp();

    // Count how many 5min markets have ended
    let closed = five_min_markets.iter()
        .filter(|m| m.end_time < now)
        .count();

    match closed {
        0 => MarketPhase::Initial,
        1 => MarketPhase::AfterFirst,
        2 => MarketPhase::AfterSecond,
        _ => MarketPhase::Complete,
    }
}

/// Build temporal chains from market data
pub fn build_temporal_chains(
    five_min_markets: Vec<MarketInfo>,
    fifteen_min_markets: Vec<MarketInfo>,
) -> Vec<TemporalChain> {
    let mut chains = Vec::new();

    for fifteen in &fifteen_min_markets {
        // Find overlapping 5min markets
        let overlapping: Vec<_> = five_min_markets.iter()
            .filter(|m| {
                m.start_time >= fifteen.start_time &&
                m.end_time <= fifteen.end_time &&
                m.start_time < fifteen.end_time
            })
            .cloned()
            .collect();

        if overlapping.len() == 3 {
            chains.push(TemporalChain {
                target_market: fifteen.clone(),
                five_min_markets: overlapping,
                reference_price: fifteen.reference_price,
                chain_type: ChainType::FiveToFifteen,
            });
        }
    }

    chains
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_five_min_market(
        condition_id: &str,
        start_offset: i64,
        end_offset: i64,
        reference: f64,
        current: Option<f64>,
    ) -> MarketInfo {
        let now = chrono::Utc::now().timestamp();
        MarketInfo {
            condition_id: condition_id.to_string(),
            question: format!("5min {}", condition_id),
            start_time: now + start_offset,
            end_time: now + end_offset,
            reference_price: reference,
            current_price: current,
            yes_price: Some(0.5),
            no_price: Some(0.5),
        }
    }

    fn mock_fifteen_min_market(
        condition_id: &str,
        start_offset: i64,
        end_offset: i64,
        reference: f64,
        yes_price: f64,
    ) -> MarketInfo {
        let now = chrono::Utc::now().timestamp();
        MarketInfo {
            condition_id: condition_id.to_string(),
            question: format!("15min {}", condition_id),
            start_time: now + start_offset,
            end_time: now + end_offset,
            reference_price: reference,
            current_price: None,
            yes_price: Some(yes_price),
            no_price: Some(1.0 - yes_price),
        }
    }

    #[test]
    fn test_initial_fifteen_prob_efficient() {
        let engine = TemporalProbabilityEngine::new(TransitionMatrix::efficient_market());
        let prob = engine.initial_fifteen_prob();
        assert!((prob - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_initial_fifteen_prob_momentum() {
        let engine = TemporalProbabilityEngine::new(TransitionMatrix::slight_momentum());
        let prob = engine.initial_fifteen_prob();
        // With momentum, should be slightly different from 0.5
        assert!(prob > 0.5 && prob < 0.6);
    }

    #[test]
    fn test_conditional_after_first_up() {
        let engine = TemporalProbabilityEngine::new(TransitionMatrix::slight_momentum());
        let prob = engine.conditional_after_first(PriceMove::Up { change: 40.0 }, 40.0);
        // After first UP, probability of 15min UP should increase
        assert!(prob > 0.5);
    }

    #[test]
    fn test_conditional_after_first_down() {
        let engine = TemporalProbabilityEngine::new(TransitionMatrix::slight_momentum());
        let prob = engine.conditional_after_first(PriceMove::Down { change: 40.0 }, 40.0);
        // After first DOWN, probability of 15min UP should decrease
        assert!(prob < 0.5);
    }

    #[test]
    fn test_build_temporal_chains() {
        let five_mins = vec![
            mock_five_min_market("5m-1", 0, 300, 71496.0, None),
            mock_five_min_market("5m-2", 300, 600, 71496.0, None),
            mock_five_min_market("5m-3", 600, 900, 71496.0, None),
        ];
        let fifteen_mins = vec![
            mock_fifteen_min_market("15m-1", 0, 900, 71496.0, 0.5),
        ];

        let chains = build_temporal_chains(five_mins, fifteen_mins);
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].five_min_markets.len(), 3);
    }

    #[test]
    fn test_generate_signals_initial() {
        let engine = TemporalProbabilityEngine::new(TransitionMatrix::slight_momentum());
        let chains = vec![TemporalChain {
            target_market: mock_fifteen_min_market("15m-1", 0, 900, 71496.0, 0.45),
            five_min_markets: vec![
                mock_five_min_market("5m-1", 0, 300, 71496.0, None),
                mock_five_min_market("5m-2", 300, 600, 71496.0, None),
                mock_five_min_market("5m-3", 600, 900, 71496.0, None),
            ],
            reference_price: 71496.0,
            chain_type: ChainType::FiveToFifteen,
        }];

        let signals = generate_signals(&chains, &engine, 0.03);
        // Should have a signal since fair prob > 0.5 and market is 0.45
        assert!(!signals.is_empty());
    }

    #[test]
    fn test_determine_phase() {
        let now = chrono::Utc::now().timestamp();
        let markets = vec![
            mock_five_min_market("past", -900, -300, 71496.0, Some(71537.0)),
            mock_five_min_market("current", -300, 300, 71537.0, None),
            mock_five_min_market("future", 300, 900, 71496.0, None),
        ];

        let phase = determine_phase(&markets);
        // One market ended (in past)
        assert_eq!(phase, MarketPhase::AfterFirst);
    }
}
