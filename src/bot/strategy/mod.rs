//! Strategy Engine Module
//!
//! Unified strategy engine for Polymarket trading decisions.

mod types;
mod heuristic;
mod risk;

pub use types::*;
pub use heuristic::HeuristicEngine;
pub use risk::RiskGate;

use crate::bot::indicators::IndicatorState;

/// Observation snapshot for strategy decision
#[derive(Debug, Clone)]
pub struct Observation {
    pub ts: i64,
    pub condition_id: String,
    pub market_slug: String,
    pub yes_bid: f64,
    pub yes_ask: f64,
    pub no_bid: f64,
    pub no_ask: f64,
    pub yes_mid: f64,
    pub no_mid: f64,
    pub book_sum: f64,
    pub time_remaining_s: i64,
    pub indicator_5s: IndicatorState,
    pub indicator_1m: IndicatorState,
    pub fair_value_prob: Option<f64>,
    pub qlib_score: Option<f64>,
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            ts: 0,
            condition_id: String::new(),
            market_slug: String::new(),
            yes_bid: 0.5,
            yes_ask: 0.5,
            no_bid: 0.5,
            no_ask: 0.5,
            yes_mid: 0.5,
            no_mid: 0.5,
            book_sum: 1.0,
            time_remaining_s: 300,
            indicator_5s: IndicatorState::default(),
            indicator_1m: IndicatorState::default(),
            fair_value_prob: None,
            qlib_score: None,
        }
    }
}

/// Core strategy engine trait
pub trait StrategyEngine {
    /// Make a trading decision based on the current observation
    fn decide(&mut self, obs: &Observation) -> StrategyDecision;

    /// Reset internal state (e.g., when switching markets)
    fn reset(&mut self);
}

/// Fused engine combining heuristic and Qlib signals
pub struct FusedEngine {
    heuristic: HeuristicEngine,
    mode: FusionMode,
    confidence_threshold: f64,
}

impl FusedEngine {
    pub fn new(mode: FusionMode) -> Self {
        Self {
            heuristic: HeuristicEngine::new(),
            mode,
            confidence_threshold: 0.5,
        }
    }

    pub fn with_confidence_threshold(mut self, threshold: f64) -> Self {
        self.confidence_threshold = threshold;
        self
    }
}

impl StrategyEngine for FusedEngine {
    fn decide(&mut self, obs: &Observation) -> StrategyDecision {
        match self.mode {
            FusionMode::HeuristicOnly => self.heuristic.decide(obs),
            FusionMode::QlibOnly => {
                // Qlib-only mode: use scores if available
                match obs.qlib_score {
                    Some(score) if score > self.confidence_threshold => {
                        StrategyDecision::Enter {
                            direction: Direction::Yes,
                            reason: EntryReason {
                                source: SignalSource::QlibScore,
                                confidence: Confidence::new(score),
                                detail: format!("Qlib score: {:.3}", score),
                                fair_value_edge: None,
                                qlib_score: Some(score),
                            },
                        }
                    }
                    Some(score) if score < -self.confidence_threshold => {
                        StrategyDecision::Enter {
                            direction: Direction::No,
                            reason: EntryReason {
                                source: SignalSource::QlibScore,
                                confidence: Confidence::new(-score),
                                detail: format!("Qlib score: {:.3}", score),
                                fair_value_edge: None,
                                qlib_score: Some(score),
                            },
                        }
                    }
                    _ => StrategyDecision::Hold,
                }
            }
            FusionMode::Fused => {
                // Fused mode: both must agree
                let heuristic_decision = self.heuristic.decide(obs);

                match (&heuristic_decision, obs.qlib_score) {
                    (StrategyDecision::Enter { direction, reason }, Some(score))
                        if score.abs() > self.confidence_threshold =>
                    {
                        // Both agree - boost confidence
                        StrategyDecision::Enter {
                            direction: direction.clone(),
                            reason: EntryReason {
                                source: SignalSource::Fused,
                                confidence: Confidence::new(
                                    (reason.confidence.value() + score.abs()) / 2.0,
                                ),
                                detail: format!(
                                    "Fused: heuristic + qlib ({:.3})",
                                    score
                                ),
                                fair_value_edge: reason.fair_value_edge,
                                qlib_score: Some(score),
                            },
                        }
                    }
                    (StrategyDecision::Enter { .. }, Some(_)) => {
                        // Qlib doesn't agree strongly enough - block
                        StrategyDecision::Hold
                    }
                    (StrategyDecision::Enter { .. }, None) => {
                        // No Qlib score available - fall back to heuristic
                        heuristic_decision
                    }
                    _ => heuristic_decision,
                }
            }
        }
    }

    fn reset(&mut self) {
        self.heuristic.reset();
    }
}
