//! Fusion Logic
//!
//! Combine heuristic and Qlib signals.

use super::{ScoreLoader, ScoreRow};
use crate::bot::strategy::{Confidence, Direction, EntryReason, SignalSource, StrategyDecision, StrategyEngine, Observation};
use serde::{Deserialize, Serialize};

/// Fusion mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FusionMode {
    #[default]
    HeuristicOnly,
    QlibOnly,
    Fused,
}

/// Fusion configuration
#[derive(Debug, Clone, Deserialize)]
pub struct FusionConfig {
    /// Minimum Qlib score threshold for entry
    pub score_threshold: f64,
    /// Minimum confidence for heuristic
    pub heuristic_threshold: f64,
    /// How much to weigh Qlib vs heuristic in fused mode
    pub qlib_weight: f64,
    /// Fallback to heuristic if score is stale
    pub fallback_on_stale: bool,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            score_threshold: 0.3,
            heuristic_threshold: 0.5,
            qlib_weight: 0.5,
            fallback_on_stale: true,
        }
    }
}

/// Fusion decision output
#[derive(Debug, Clone)]
pub struct FusionDecision {
    pub direction: Direction,
    pub confidence: f64,
    pub source: SignalSource,
    pub heuristic_signal: Option<String>,
    pub qlib_score: Option<f64>,
    pub fallback_reason: Option<String>,
}

/// Fusion engine combining multiple signal sources
pub struct FusionEngine {
    mode: FusionMode,
    config: FusionConfig,
    score_loader: ScoreLoader,
}

impl FusionEngine {
    pub fn new(mode: FusionMode, config: FusionConfig) -> Self {
        Self {
            mode,
            config,
            score_loader: ScoreLoader::default(),
        }
    }

    /// Set fusion mode
    pub fn set_mode(&mut self, mode: FusionMode) {
        self.mode = mode;
    }

    /// Load scores from file
    pub fn load_scores(&mut self, path: &std::path::Path) -> anyhow::Result<usize> {
        self.score_loader.load(path)
    }

    /// Make a fused decision
    pub fn decide(
        &self,
        heuristic_decision: &StrategyDecision,
        obs: &Observation,
    ) -> FusionDecision {
        match self.mode {
            FusionMode::HeuristicOnly => self.heuristic_only(heuristic_decision),
            FusionMode::QlibOnly => self.qlib_only(obs),
            FusionMode::Fused => self.fused(heuristic_decision, obs),
        }
    }

    fn heuristic_only(&self, decision: &StrategyDecision) -> FusionDecision {
        match decision {
            StrategyDecision::Enter { direction, reason } => FusionDecision {
                direction: direction.clone(),
                confidence: reason.confidence.value(),
                source: reason.source,
                heuristic_signal: Some(reason.detail.clone()),
                qlib_score: None,
                fallback_reason: None,
            },
            StrategyDecision::Exit { reason, .. } => FusionDecision {
                direction: Direction::Yes, // Default
                confidence: 1.0,
                source: SignalSource::Indicators,
                heuristic_signal: Some(format!("{:?}", reason)),
                qlib_score: None,
                fallback_reason: None,
            },
            _ => FusionDecision {
                direction: Direction::Yes,
                confidence: 0.0,
                source: SignalSource::Indicators,
                heuristic_signal: None,
                qlib_score: None,
                fallback_reason: None,
            },
        }
    }

    fn qlib_only(&self, obs: &Observation) -> FusionDecision {
        match &obs.qlib_score {
            Some(score) if *score > self.config.score_threshold => FusionDecision {
                direction: Direction::Yes,
                confidence: score.abs(),
                source: SignalSource::QlibScore,
                heuristic_signal: None,
                qlib_score: Some(*score),
                fallback_reason: None,
            },
            Some(score) if *score < -self.config.score_threshold => FusionDecision {
                direction: Direction::No,
                confidence: score.abs(),
                source: SignalSource::QlibScore,
                heuristic_signal: None,
                qlib_score: Some(*score),
                fallback_reason: None,
            },
            Some(_) => FusionDecision {
                direction: Direction::Yes,
                confidence: 0.0,
                source: SignalSource::QlibScore,
                heuristic_signal: None,
                qlib_score: Some(0.0),
                fallback_reason: Some("Score below threshold".to_string()),
            },
            None => FusionDecision {
                direction: Direction::Yes,
                confidence: 0.0,
                source: SignalSource::QlibScore,
                heuristic_signal: None,
                qlib_score: None,
                fallback_reason: Some("No score available".to_string()),
            },
        }
    }

    fn fused(&self, heuristic_decision: &StrategyDecision, obs: &Observation) -> FusionDecision {
        let heuristic = self.heuristic_only(heuristic_decision);
        let qlib = self.qlib_only(obs);

        // Both must agree for entry
        if heuristic.confidence > self.config.heuristic_threshold
            && qlib.qlib_score.map(|s| s.abs() > self.config.score_threshold).unwrap_or(false)
        {
            // Check if directions agree
            let directions_agree = match heuristic_decision {
                StrategyDecision::Enter { direction, .. } => {
                    *direction == qlib.direction || qlib.confidence < self.config.score_threshold
                }
                _ => false,
            };

            if directions_agree {
                // Fused confidence
                let fused_confidence = (heuristic.confidence * (1.0 - self.config.qlib_weight)
                    + qlib.confidence * self.config.qlib_weight);

                return FusionDecision {
                    direction: heuristic.direction,
                    confidence: fused_confidence,
                    source: SignalSource::Fused,
                    heuristic_signal: heuristic.heuristic_signal,
                    qlib_score: qlib.qlib_score,
                    fallback_reason: None,
                };
            }
        }

        // Fallback logic
        if self.config.fallback_on_stale && heuristic.confidence > self.config.heuristic_threshold {
            return FusionDecision {
                fallback_reason: Some("Qlib disagree/stale, using heuristic".to_string()),
                ..heuristic
            };
        }

        // No agreement - hold
        FusionDecision {
            direction: Direction::Yes,
            confidence: 0.0,
            source: SignalSource::Fused,
            heuristic_signal: heuristic.heuristic_signal,
            qlib_score: qlib.qlib_score,
            fallback_reason: Some("No agreement between signals".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_only_mode() {
        let engine = FusionEngine::new(FusionMode::HeuristicOnly, FusionConfig::default());
        let decision = StrategyDecision::Enter {
            direction: Direction::Yes,
            reason: EntryReason {
                source: SignalSource::Indicators,
                confidence: Confidence::new(0.7),
                detail: "Test".to_string(),
                fair_value_edge: None,
                qlib_score: None,
            },
        };

        let result = engine.decide(&decision, &Default::default());
        assert_eq!(result.source, SignalSource::Indicators);
        assert!((result.confidence - 0.7).abs() < 0.01);
    }

    #[test]
    fn qlib_only_mode_with_positive_score() {
        let engine = FusionEngine::new(FusionMode::QlibOnly, FusionConfig::default());
        let obs = Observation {
            qlib_score: Some(0.5),
            ..Default::default()
        };

        let result = engine.decide(&StrategyDecision::Hold, &obs);
        assert_eq!(result.direction, Direction::Yes);
        assert_eq!(result.source, SignalSource::QlibScore);
    }

    #[test]
    fn qlib_only_mode_with_negative_score() {
        let engine = FusionEngine::new(FusionMode::QlibOnly, FusionConfig::default());
        let obs = Observation {
            qlib_score: Some(-0.5),
            ..Default::default()
        };

        let result = engine.decide(&StrategyDecision::Hold, &obs);
        assert_eq!(result.direction, Direction::No);
    }
}
