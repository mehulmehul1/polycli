//! Temporal Fusion for Fair Value + Temporal Arbitrage Integration
//!
//! Combines risk-neutral pricing model (Fair Value) with information flow
//! advantages from Temporal Arbitrage to create superior trading signals.

use crate::bot::market_classifier::{MarketHorizon, HorizonParams};
use crate::bot::pricing::logit_model::{prob_to_logit};
use crate::bot::research::temporal_arbitrage::{
    MarketPhase, PriceMove, TemporalChain, TransitionMatrix,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Multi-horizon fair value with temporal integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiHorizonFairValue {
    /// Base risk-neutral fair value
    pub risk_neutral_fv: f64,

    /// Temporal arbitrage adjusted probability
    pub temporal_adjusted_fv: f64,

    /// Confidence in combined estimate (0-1)
    pub confidence: f64,

    /// Breakdown by horizon
    pub by_horizon: HashMap<String, HorizonFairValue>,

    /// Temporal information used
    pub temporal_info: Option<TemporalObservation>,
}

/// Fair value for a specific market horizon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HorizonFairValue {
    /// Market identifier
    pub market_id: String,

    /// Fair probability for this horizon
    pub fair_prob: f64,

    /// Market price for this horizon
    pub market_prob: f64,

    /// Edge (fair - market)
    pub edge: f64,

    /// Weight in final decision (0-1)
    pub weight: f64,

    /// Temporal phase (for chained markets)
    pub temporal_phase: TemporalPhaseSummary,
}

/// Summary of temporal phase for a market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalPhaseSummary {
    /// Current phase
    pub phase: MarketPhase,

    /// Observed moves so far
    pub observed_moves: Vec<PriceMoveSummary>,

    /// Cumulative price change from reference
    pub cumulative_change: f64,

    /// Information quality (0-1, higher = better)
    pub information_quality: f64,
}

/// Summary of a price move
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceMoveSummary {
    /// Direction and magnitude
    pub direction: MoveDirection,
    /// Change amount
    pub magnitude: f64,
    /// Timestamp of move
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MoveDirection {
    Up,
    Down,
    Flat,
}

/// Temporal observation from a chain of markets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalObservation {
    /// Phase in the temporal chain
    pub phase: MarketPhase,

    /// Predicted logit based on temporal information
    pub predicted_logit: f64,

    /// Uncertainty in prediction (0-1)
    pub uncertainty: f64,

    /// Quality of information (0-1)
    pub quality: f64,

    /// Observed moves so far
    pub observed_moves: Vec<PriceMove>,

    /// Cumulative price change
    pub cumulative_change: f64,
}

/// Signal fusion configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusionConfig {
    /// Weight for fair value signal (0-1)
    pub fair_value_weight: f64,

    /// Weight for temporal signal (0-1)
    pub temporal_weight: f64,

    /// Minimum confidence threshold
    pub min_confidence: f64,

    /// High confidence threshold
    pub high_confidence_threshold: f64,

    /// Regime adjustment factors
    pub regime_tight_multiplier: f64,
    pub regime_normal_multiplier: f64,
    pub regime_reversal_multiplier: f64,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            fair_value_weight: 0.7,
            temporal_weight: 0.3,
            min_confidence: 0.3,
            high_confidence_threshold: 0.15,
            regime_tight_multiplier: 1.2,
            regime_normal_multiplier: 1.0,
            regime_reversal_multiplier: 0.5,
        }
    }
}

/// Market regime state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Regime {
    /// Tight spread, low volatility - good entry conditions
    Tight,
    /// Normal market conditions
    Normal,
    /// Wide spread, high volatility - choppy/reversal
    Reversal,
}

/// Fused signal from combining multiple sources
#[derive(Debug, Clone)]
pub struct FusedSignal {
    /// Final combined edge
    pub edge: f64,

    /// Overall confidence (0-1)
    pub confidence: f64,

    /// Recommended action
    pub action: SignalAction,

    /// Component breakdown
    pub components: SignalComponents,

    /// Current regime
    pub regime: Regime,

    /// Horizon classification
    pub horizon: MarketHorizon,
}

#[derive(Debug, Clone)]
pub struct SignalComponents {
    /// Fair value model edge
    pub fair_value_edge: f64,

    /// Temporal arbitrage edge
    pub temporal_edge: f64,

    /// Regime adjustment factor applied
    pub regime_factor: f64,

    /// Horizon multiplier applied
    pub horizon_multiplier: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalAction {
    BuyYes,
    BuyNo,
    Hold,
}

/// Temporal fusion engine
pub struct TemporalFusionEngine {
    /// Fusion configuration
    config: FusionConfig,

    /// Transition probabilities for temporal prediction
    transitions: TransitionMatrix,

    /// Historical temporal patterns
    temporal_history: Vec<TemporalPattern>,
}

#[derive(Debug, Clone)]
struct TemporalPattern {
    first_move: PriceMove,
    second_move: Option<PriceMove>,
    final_outcome: PriceMove,
    volatility: f64,
}

impl TemporalFusionEngine {
    pub fn new(config: FusionConfig, transitions: TransitionMatrix) -> Self {
        Self {
            config,
            transitions,
            temporal_history: Vec::new(),
        }
    }

    /// Calculate temporal adjustment to fair value
    pub fn temporal_adjustment(
        &self,
        base_fv: f64,
        temporal_obs: &TemporalObservation,
        time_remaining: i64,
        horizon: &MarketHorizon,
    ) -> (f64, f64) {
        // Returns (adjusted_fv, confidence)

        match temporal_obs.phase {
            MarketPhase::Initial => {
                // No temporal info yet
                (base_fv, 0.5)
            }
            MarketPhase::AfterFirst => {
                let adjustment = self.adjustment_after_first(
                    &temporal_obs.observed_moves[0],
                    temporal_obs.cumulative_change,
                    time_remaining,
                );

                let adjusted = (base_fv + adjustment).clamp(0.01, 0.99);
                let confidence = 0.5 + (adjustment.abs() * 2.0).min(0.4);

                (adjusted, confidence)
            }
            MarketPhase::AfterSecond => {
                let adjustment = self.adjustment_after_second(
                    &temporal_obs.observed_moves[0],
                    &temporal_obs.observed_moves[1],
                    temporal_obs.cumulative_change,
                    time_remaining,
                );

                let adjusted = (base_fv + adjustment).clamp(0.01, 0.99);
                let confidence = 0.7 + (adjustment.abs() * 2.0).min(0.25);

                (adjusted, confidence)
            }
            MarketPhase::Complete => {
                (base_fv, 0.0)
            }
        }
    }

    /// Calculate adjustment after first 5min result
    fn adjustment_after_first(
        &self,
        first_move: &PriceMove,
        magnitude: f64,
        time_remaining: i64,
    ) -> f64 {
        // Normalize magnitude (assume $50 is "large" move)
        let signal_strength = (magnitude / 50.0).min(1.0);

        // Time factor (more info as we get closer to end)
        let total_time = 900.0;  // 15 min
        let time_factor = 1.0 - (time_remaining as f64 / total_time);

        // Base adjustment depends on direction
        let base_adjustment = match first_move {
            PriceMove::Up { .. } => 0.15,
            PriceMove::Down { .. } => -0.15,
            PriceMove::Flat => 0.0,
        };

        // Scale by signal strength and time factor
        base_adjustment * signal_strength * time_factor
    }

    /// Calculate adjustment after second 5min result
    fn adjustment_after_second(
        &self,
        first_move: &PriceMove,
        second_move: &PriceMove,
        cumulative: f64,
        time_remaining: i64,
    ) -> f64 {
        // After two moves, cumulative position is key
        // If cumulative > $20, very likely to stay UP
        // If cumulative < -$20, very likely to stay DOWN

        let signal_strength = (cumulative.abs() / 30.0).min(1.0);

        // Time factor (very high now, only 5min left)
        let time_factor = 1.0 - (time_remaining as f64 / 900.0);

        // Base adjustment based on cumulative
        let base_adjustment = if cumulative > 20.0 {
            0.25  // Strong UP signal
        } else if cumulative < -20.0 {
            -0.25  // Strong DOWN signal
        } else if cumulative > 0.0 {
            0.10  // Slight UP
        } else if cumulative < 0.0 {
            -0.10  // Slight DOWN
        } else {
            0.0
        };

        // Consider momentum vs reversal
        let momentum_factor = match (first_move, second_move) {
            (PriceMove::Up { .. }, PriceMove::Up { .. }) => 1.2,   // Momentum
            (PriceMove::Down { .. }, PriceMove::Down { .. }) => 1.2,  // Momentum
            _ => 0.8,  // Potential reversal
        };

        base_adjustment * signal_strength * time_factor * momentum_factor
    }

    /// Create temporal observation from chain state
    pub fn create_temporal_observation(
        &self,
        chain: &TemporalChain,
        current_time: i64,
    ) -> Option<TemporalObservation> {
        // Determine current phase
        let phase = self.determine_phase(chain, current_time);

        // Collect observed moves
        let mut observed_moves = Vec::new();
        let mut cumulative_change = 0.0;

        for market in &chain.five_min_markets {
            if market.end_time < current_time {
                // This market has closed
                if let Some(final_price) = market.current_price {
                    let change = final_price - chain.reference_price;
                    cumulative_change = final_price - chain.reference_price;

                    let price_move = if change > 0.5 {
                        PriceMove::Up { change }
                    } else if change < -0.5 {
                        PriceMove::Down { change: change.abs() }
                    } else {
                        PriceMove::Flat
                    };

                    observed_moves.push(price_move);
                }
            }
        }

        if phase == MarketPhase::Initial {
            return None;
        }

        // Calculate information quality
        let quality = self.information_quality(&observed_moves, &phase);

        // Predict logit based on temporal info
        let predicted_logit = match phase {
            MarketPhase::AfterFirst => {
                // After first move, adjust probability
                if let Some(first) = observed_moves.first() {
                    let base_prob = 0.5;
                    let adjustment = self.adjustment_after_first(
                        first,
                        first.change(),
                        600,  // 10 min remaining
                    );
                    prob_to_logit(base_prob + adjustment)
                } else {
                    0.0
                }
            }
            MarketPhase::AfterSecond => {
                // After two moves, stronger prediction
                if observed_moves.len() >= 2 {
                    let base_prob = 0.5;
                    let adjustment = self.adjustment_after_second(
                        &observed_moves[0],
                        &observed_moves[1],
                        cumulative_change,
                        300,  // 5 min remaining
                    );
                    prob_to_logit(base_prob + adjustment)
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };

        // Uncertainty decreases with more information
        let uncertainty = match phase {
            MarketPhase::AfterFirst => 0.4,
            MarketPhase::AfterSecond => 0.2,
            _ => 0.5,
        };

        Some(TemporalObservation {
            phase,
            predicted_logit,
            uncertainty,
            quality,
            observed_moves,
            cumulative_change,
        })
    }

    /// Determine current phase of a temporal chain
    fn determine_phase(&self, chain: &TemporalChain, current_time: i64) -> MarketPhase {
        let closed_count = chain.five_min_markets
            .iter()
            .filter(|m| m.end_time < current_time)
            .count();

        match closed_count {
            0 => MarketPhase::Initial,
            1 => MarketPhase::AfterFirst,
            2 => MarketPhase::AfterSecond,
            _ => MarketPhase::Complete,
        }
    }

    /// Calculate information quality of temporal observations
    fn information_quality(&self, moves: &[PriceMove], phase: &MarketPhase) -> f64 {
        if moves.is_empty() {
            return 0.0;
        }

        // Quality increases with:
        // 1. Number of observations
        // 2. Consistency of moves (momentum)
        // 3. Magnitude of moves (signal vs noise)

        let count_bonus = match phase {
            MarketPhase::AfterFirst => 0.3,
            MarketPhase::AfterSecond => 0.6,
            _ => 0.0,
        };

        // Check for momentum (consistent direction)
        let momentum_bonus = if moves.len() >= 2 {
            let consistent = moves.iter().all(|m| m.is_up())
                || moves.iter().all(|m| !m.is_up());
            if consistent { 0.2 } else { 0.0 }
        } else {
            0.0
        };

        // Magnitude bonus
        let magnitude_bonus = moves.iter()
            .map(|m| (m.change() / 50.0).min(0.1))
            .sum::<f64>();

        (count_bonus + momentum_bonus + magnitude_bonus).min(1.0)
    }

    /// Fuse fair value and temporal signals
    pub fn fuse_signals(
        &self,
        fair_value_prob: f64,
        market_prob: f64,
        temporal_obs: Option<&TemporalObservation>,
        regime: Regime,
        horizon: &MarketHorizon,
    ) -> FusedSignal {
        // Calculate fair value edge
        let fv_edge = fair_value_prob - market_prob;

        // Calculate temporal edge if available
        let (temporal_fv, temporal_confidence) = match temporal_obs {
            None => (fair_value_prob, 0.0),
            Some(obs) => {
                let time_remaining = match horizon {
                    MarketHorizon::UltraShort => 300,
                    MarketHorizon::Short => 600,
                    MarketHorizon::Medium => 3600,
                    MarketHorizon::Long => 86400,
                };
                self.temporal_adjustment(fair_value_prob, obs, time_remaining, horizon)
            }
        };

        let temporal_edge = temporal_fv - market_prob;

        // Apply regime adjustment
        let regime_factor = match regime {
            Regime::Tight => self.config.regime_tight_multiplier,
            Regime::Normal => self.config.regime_normal_multiplier,
            Regime::Reversal => self.config.regime_reversal_multiplier,
        };

        // Fuse edges
        let base_edge = fv_edge * self.config.fair_value_weight
            + temporal_edge * self.config.temporal_weight * temporal_confidence;

        let mut edge = base_edge * regime_factor;

        // Apply horizon multiplier
        let horizon_multiplier = horizon.edge_multiplier();
        edge *= horizon_multiplier;

        // Calculate final confidence
        let base_confidence = fv_edge.abs() / 0.10;  // Normalize by max edge
        let confidence = (base_confidence * (1.0 - temporal_confidence)
            + temporal_confidence * temporal_confidence)
            .clamp(0.0, 1.0);

        // Determine action
        let action = if edge > self.config.min_confidence {
            SignalAction::BuyYes
        } else if edge < -self.config.min_confidence {
            SignalAction::BuyNo
        } else {
            SignalAction::Hold
        };

        FusedSignal {
            edge,
            confidence,
            action,
            components: SignalComponents {
                fair_value_edge: fv_edge,
                temporal_edge,
                regime_factor,
                horizon_multiplier,
            },
            regime,
            horizon: *horizon,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adjustment_after_first_up() {
        let engine = TemporalFusionEngine::new(
            FusionConfig::default(),
            TransitionMatrix::efficient_market(),
        );

        let adjustment = engine.adjustment_after_first(
            &PriceMove::Up { change: 40.0 },
            40.0,
            600,  // 10 min remaining
        );

        // Should be positive adjustment
        assert!(adjustment > 0.0);
        assert!(adjustment < 0.2);  // Not too large
    }

    #[test]
    fn test_adjustment_after_second_strong_up() {
        let engine = TemporalFusionEngine::new(
            FusionConfig::default(),
            TransitionMatrix::slight_momentum(),
        );

        let adjustment = engine.adjustment_after_second(
            &PriceMove::Up { change: 30.0 },
            &PriceMove::Up { change: 20.0 },
            50.0,  // Cumulative +$50
            300,  // 5 min remaining
        );

        // Strong UP signal expected
        assert!(adjustment > 0.2);
    }

    #[test]
    fn test_fuse_signals_no_temporal() {
        let engine = TemporalFusionEngine::new(
            FusionConfig::default(),
            TransitionMatrix::efficient_market(),
        );

        let fused = engine.fuse_signals(
            0.55,  // Fair value
            0.50,  // Market price
            None,  // No temporal info
            Regime::Normal,
            &MarketHorizon::Medium,
        );

        // Should have positive edge
        assert!(fused.edge > 0.0);
        assert_eq!(fused.action, SignalAction::BuyYes);
    }

    #[test]
    fn test_fuse_signals_with_temporal() {
        let engine = TemporalFusionEngine::new(
            FusionConfig::default(),
            TransitionMatrix::slight_momentum(),
        );

        let temporal_obs = TemporalObservation {
            phase: MarketPhase::AfterSecond,
            predicted_logit: prob_to_logit(0.65),
            uncertainty: 0.2,
            quality: 0.8,
            observed_moves: vec![
                PriceMove::Up { change: 40.0 },
                PriceMove::Up { change: 15.0 },
            ],
            cumulative_change: 55.0,
        };

        let fused = engine.fuse_signals(
            0.52,  // Fair value
            0.48,  // Market price
            Some(&temporal_obs),
            Regime::Normal,
            &MarketHorizon::Short,
        );

        // Should have strong positive edge
        assert!(fused.edge > 0.05);
        assert_eq!(fused.action, SignalAction::BuyYes);
    }
}
