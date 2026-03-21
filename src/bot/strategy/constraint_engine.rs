//! Constraint Engine for Temporal Arbitrage
//!
//! This module detects constraint violations in the temporal market graph
//! that represent arbitrage opportunities.
//!
//! Key violations:
//! - Chained conditional arbitrage (PRIMARY EDGE)
//! - Late-phase pricing anomalies
//! - Price consistency violations

use crate::bot::strategy::graph_builder::TemporalGraph;
use crate::bot::strategy::probability_engine::{
    ArbitrageAction, ArbitrageActionReason, ProbabilityEngine, ViolationType,
};
use crate::bot::strategy::temporal_arbitrage::TemporalArbitrageConfig;
use serde::{Deserialize, Serialize};

/// Constraint engine configuration
#[derive(Debug, Clone)]
pub struct ConstraintConfig {
    /// Minimum edge for chained conditional
    pub min_chained_edge: f64,
    /// Minimum edge for late phase
    pub min_late_phase_edge: f64,
    /// Minimum edge for price consistency
    pub min_price_consistency_edge: f64,
    /// Late phase threshold (seconds)
    pub late_phase_threshold_sec: i64,
    /// Maximum spread allowed
    pub max_spread: f64,
}

impl Default for ConstraintConfig {
    fn default() -> Self {
        Self {
            min_chained_edge: 0.05,
            min_late_phase_edge: 0.08,
            min_price_consistency_edge: 0.06,
            late_phase_threshold_sec: 120,
            max_spread: 0.03,
        }
    }
}

/// Constraint violation detected
#[derive(Debug, Clone)]
pub struct ConstraintViolation {
    /// Type of violation
    pub violation_type: ViolationType,
    /// Nodes involved in the violation
    pub nodes: Vec<String>,
    /// Expected fair probability
    pub expected: f64,
    /// Actual market price
    pub actual: f64,
    /// Edge (difference)
    pub edge: f64,
    /// Confidence in this violation
    pub confidence: f64,
    /// Action to take
    pub action: ArbitrageAction,
}

/// Constraint engine for detecting arbitrage opportunities
pub struct ConstraintEngine {
    config: ConstraintConfig,
}

impl ConstraintEngine {
    /// Create a new constraint engine
    pub fn new(config: TemporalArbitrageConfig) -> Self {
        Self {
            config: ConstraintConfig {
                min_chained_edge: config.min_edge,
                min_late_phase_edge: config.min_edge,
                min_price_consistency_edge: config.min_edge,
                late_phase_threshold_sec: 120,
                max_spread: config.max_spread,
            },
        }
    }

    /// CORRECTED: Check chained conditional after some children resolve
    ///
    /// This is the PRIMARY EDGE source. After k of n children resolve,
    /// we observe the actual price path. The parent's fair probability
    /// becomes:
    ///
    /// P(parent=YES | observed path) = Φ((P_live - P_original) / (σ * √t_remaining))
    ///
    /// # Example
    /// ```text
    /// 15min market (1:15-1:30PM), strike = $71,496:
    ///   ├─ 5m #1 (1:15-1:20): close = $71,550 (YES)
    ///   ├─ 5m #2 (1:20-1:25): close = $71,530 (NO)
    ///   └─ 5m #3 (1:25-1:30): pending
    ///
    /// Live BTC: $71,530, need P₃ > P₀ = $71,496
    /// Market prices at 0.40, but fair ≈ 0.60 (edge = 20%)
    /// ```
    pub fn check_chained_conditional(
        &self,
        graph: &TemporalGraph,
        parent_id: &str,
        now: i64,
    ) -> Option<ArbitrageAction> {
        let parent = graph.nodes.get(parent_id)?;

        // Get children
        let children: Vec<_> = parent
            .children
            .iter()
            .filter_map(|id| graph.nodes.get(id))
            .collect();

        if children.is_empty() {
            return None;
        }

        // Find resolved and unresolved children
        let resolved: Vec<_> = children.iter().filter(|c| c.is_resolved(now)).collect();

        let unresolved: Vec<_> = children.iter().filter(|c| c.is_active(now)).collect();

        // Need at least one resolved and one unresolved child
        if resolved.is_empty() || unresolved.is_empty() {
            return None;
        }

        // Extract observed legs
        let mut observed_legs = Vec::new();
        for child in &resolved {
            let close_price = child.close_price?;
            let outcome = child.resolved_outcome?;
            observed_legs.push((close_price, outcome));
        }

        // FIX #2: Use live BTC price from graph state
        let live_price = graph.price_state.current_price;
        if live_price <= 0.0 {
            return None;
        }

        let remaining_time = parent.time_remaining(now) as f64;
        if remaining_time <= 0.0 {
            return None;
        }

        // Calculate fair probability using chained conditional
        let fair_prob = ProbabilityEngine::chained_conditional_after_observed_legs(
            &observed_legs,
            parent.strike_price,
            live_price, // FIX: Always use fresh spot
            remaining_time,
            graph.vol_estimator.vol_per_second(),
            0.0,
        );

        // Get market price
        let market_prob = parent.yes_price.unwrap_or(0.5);
        let edge = (fair_prob - market_prob).abs();

        if edge < self.config.min_chained_edge {
            return None;
        }

        let direction = if fair_prob > market_prob {
            super::Direction::Yes
        } else {
            super::Direction::No
        };

        // Confidence increases with more resolved legs
        let confidence = 0.5 + (resolved.len() as f64 / children.len() as f64) * 0.4;

        Some(ArbitrageAction::EnterSingle {
            condition_id: parent_id.to_string(),
            direction,
            edge,
            reason: ArbitrageActionReason::ChainedConditional(confidence),
        })
    }

    /// Late-phase anomaly detection
    ///
    /// In the last 2 minutes, price should converge to certainty
    /// based on current BTC vs strike.
    ///
    /// This catches cases where the market hasn't updated to reflect
    /// the nearly-certain outcome.
    pub fn check_late_phase_anomalies(
        &self,
        graph: &TemporalGraph,
        now: i64,
    ) -> Vec<ArbitrageAction> {
        let mut violations = Vec::new();

        for (id, node) in &graph.nodes {
            let remaining = node.time_remaining(now);

            // Check if in late phase
            if remaining > 0 && remaining <= self.config.late_phase_threshold_sec {
                let Some(yes_price) = node.yes_price else {
                    continue;
                };
                let Some(no_price) = node.no_price else {
                    continue;
                };

                // Check spread
                let spread = yes_price + no_price - 1.0;
                if spread.abs() > self.config.max_spread {
                    continue; // Skip if spread is too wide
                }

                // Calculate fair probability
                let fair_prob = ProbabilityEngine::prob_up(
                    graph.price_state.current_price,
                    node.strike_price,
                    remaining as f64,
                    graph.vol_estimator.vol_per_second(),
                    0.0,
                );

                // Adjust threshold based on remaining time
                // As time runs out, we require less edge (market should converge)
                let time_factor = remaining as f64 / self.config.late_phase_threshold_sec as f64;
                let threshold = self.config.min_late_phase_edge * time_factor;

                let edge = (fair_prob - yes_price).abs();

                if edge > threshold {
                    let direction = if fair_prob > yes_price {
                        super::Direction::Yes
                    } else {
                        super::Direction::No
                    };

                    // Higher confidence closer to expiry
                    let confidence = 0.7 + (1.0 - time_factor) * 0.25;

                    violations.push(ArbitrageAction::EnterSingle {
                        condition_id: id.clone(),
                        direction,
                        edge,
                        reason: ArbitrageActionReason::LatePhaseAnomaly(confidence),
                    });
                }
            }
        }

        violations
    }

    /// Current price consistency check
    ///
    /// For each active market, verify that P_up = Φ((BTC - strike) / (σ * √t))
    ///
    /// This catches markets that are mispriced relative to the current
    /// BTC price and time remaining.
    pub fn check_price_consistency(&self, graph: &TemporalGraph, now: i64) -> Vec<ArbitrageAction> {
        let mut violations = Vec::new();

        if graph.price_state.current_price <= 0.0 {
            return violations;
        }

        // Check if price is stale
        if graph.price_state.is_stale(now, 300) {
            return violations;
        }

        for (id, node) in &graph.nodes {
            if !node.is_active(now) {
                continue;
            }

            let Some(yes_price) = node.yes_price else {
                continue;
            };
            let Some(no_price) = node.no_price else {
                continue;
            };

            // Check spread
            let spread = yes_price + no_price - 1.0;
            if spread.abs() > self.config.max_spread {
                continue;
            }

            // Calculate fair probability
            let remaining = node.time_remaining(now) as f64;
            let fair_prob = ProbabilityEngine::prob_up(
                graph.price_state.current_price,
                node.strike_price,
                remaining,
                graph.vol_estimator.vol_per_second(),
                0.0,
            );

            let edge = (fair_prob - yes_price).abs();

            if edge >= self.config.min_price_consistency_edge {
                let direction = if fair_prob > yes_price {
                    super::Direction::Yes
                } else {
                    super::Direction::No
                };

                // Base confidence on edge size and time remaining
                let time_confidence = (remaining / 3600.0).min(1.0);
                let edge_confidence = (edge / 0.2).min(1.0);
                let confidence = 0.5 + time_confidence * 0.2 + edge_confidence * 0.2;

                violations.push(ArbitrageAction::EnterSingle {
                    condition_id: id.clone(),
                    direction,
                    edge,
                    reason: ArbitrageActionReason::PriceConsistency(confidence),
                });
            }
        }

        violations
    }

    /// Check all constraints for a given graph
    pub fn check_all(
        &self,
        graph: &TemporalGraph,
        now: i64,
        enable_chained: bool,
        enable_late_phase: bool,
        enable_consistency: bool,
    ) -> Vec<ArbitrageAction> {
        let mut violations = Vec::new();

        // Chained conditional (PRIMARY EDGE)
        if enable_chained {
            for root in &graph.roots {
                if let Some(v) = self.check_chained_conditional(graph, root, now) {
                    violations.push(v);
                }
            }
        }

        // Late-phase anomalies
        if enable_late_phase {
            violations.extend(self.check_late_phase_anomalies(graph, now));
        }

        // Price consistency
        if enable_consistency {
            violations.extend(self.check_price_consistency(graph, now));
        }

        violations
    }

    /// Calculate the expected value of a bet
    ///
    /// # Arguments
    /// * `fair_prob` - Fair value probability
    /// * `market_ask` - Market ask price
    /// * `payout` - Payout multiplier (usually 1.0)
    ///
    /// # Returns
    /// Expected value per dollar bet
    pub fn expected_value(fair_prob: f64, market_ask: f64, payout: f64) -> f64 {
        let win_prob = fair_prob;
        let lose_prob = 1.0 - fair_prob;

        win_prob * payout / market_ask - lose_prob
    }

    /// Calculate Kelly criterion position size
    ///
    /// # Arguments
    /// * `fair_prob` - Fair value probability
    /// * `market_ask` - Market ask price
    ///
    /// # Returns
    /// Fraction of bankroll to bet
    pub fn kelly_fraction(fair_prob: f64, market_ask: f64) -> f64 {
        let win_prob = fair_prob;
        let lose_prob = 1.0 - fair_prob;

        // For even money payout (simplified)
        let odds = 1.0 / market_ask - 1.0;
        let b = odds; // Net odds
        let p = win_prob;
        let q = lose_prob;

        let kelly = (b * p - q) / b;
        kelly.max(0.0) // Never bet on negative EV
    }

    /// Check if a spread is acceptable
    pub fn is_spread_acceptable(
        &self,
        yes_bid: f64,
        yes_ask: f64,
        no_bid: f64,
        no_ask: f64,
    ) -> bool {
        let yes_spread = yes_ask - yes_bid;
        let no_spread = no_ask - no_bid;
        let sum_check = (yes_ask + no_ask - 1.0).abs();

        yes_spread < self.config.max_spread
            && no_spread < self.config.max_spread
            && sum_check < self.config.max_spread
    }

    /// Check if a market is in a "broken" state
    ///
    /// A market is broken if:
    /// - yes_ask + no_ask deviates significantly from 1.0
    /// - Either spread is excessively wide
    pub fn is_market_broken(&self, yes_bid: f64, yes_ask: f64, no_bid: f64, no_ask: f64) -> bool {
        let yes_spread = yes_ask - yes_bid;
        let no_spread = no_ask - no_bid;
        let sum_check = (yes_ask + no_ask - 1.0).abs();

        yes_spread > self.config.max_spread * 3.0
            || no_spread > self.config.max_spread * 3.0
            || sum_check > self.config.max_spread * 3.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::strategy::temporal_arbitrage::{TemporalNode, Timeframe};

    fn create_test_graph() -> TemporalGraph {
        use crate::bot::strategy::temporal_arbitrage::{PriceState, VolatilityEstimator};
        use std::collections::HashMap;

        let mut graph = TemporalGraph {
            nodes: HashMap::new(),
            roots: vec![],
            price_state: PriceState {
                current_price: 71500.0,
                last_update: 1000,
                realized_vol: 500.0,
                price_path: Default::default(),
            },
            vol_estimator: VolatilityEstimator::new(250.0, 0.94),
            last_update: 1000,
        };

        // Add parent (15min)
        let parent = TemporalNode {
            condition_id: "parent".to_string(),
            timeframe: Timeframe::M15,
            strike_price: 71400.0,
            start_time: 1000,
            end_time: 2800,
            parent: None,
            children: vec![
                "child1".to_string(),
                "child2".to_string(),
                "child3".to_string(),
            ],
            yes_price: Some(0.55),
            no_price: Some(0.45),
            resolved_outcome: None,
            close_price: None,
        };
        graph.nodes.insert("parent".to_string(), parent);
        graph.roots.push("parent".to_string());

        // Add resolved child
        let child1 = TemporalNode {
            condition_id: "child1".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71400.0,
            start_time: 1000,
            end_time: 1300,
            parent: Some("parent".to_string()),
            children: vec![],
            yes_price: None,
            no_price: None,
            resolved_outcome: Some(true),
            close_price: Some(71450.0),
        };
        graph.nodes.insert("child1".to_string(), child1);

        // Add another resolved child
        let child2 = TemporalNode {
            condition_id: "child2".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71450.0,
            start_time: 1300,
            end_time: 1600,
            parent: Some("parent".to_string()),
            children: vec![],
            yes_price: None,
            no_price: None,
            resolved_outcome: Some(false),
            close_price: Some(71430.0),
        };
        graph.nodes.insert("child2".to_string(), child2);

        // Add active child
        let child3 = TemporalNode {
            condition_id: "child3".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71430.0,
            start_time: 1600,
            end_time: 1900,
            parent: Some("parent".to_string()),
            children: vec![],
            yes_price: Some(0.50),
            no_price: Some(0.50),
            resolved_outcome: None,
            close_price: None,
        };
        graph.nodes.insert("child3".to_string(), child3);

        graph
    }

    #[test]
    fn test_check_chained_conditional() {
        let engine = ConstraintEngine::new(Default::default());
        let graph = create_test_graph();
        let now = 1700;

        let action = engine.check_chained_conditional(&graph, "parent", now);

        // Should detect an edge since live price (71500) > parent strike (71400)
        // but market prices at 0.55
        assert!(action.is_some());
    }

    #[test]
    fn test_check_late_phase_anomalies() {
        let engine = ConstraintEngine::new(Default::default());
        let mut graph = create_test_graph();

        // Set up a node in late phase with mispricing
        let now = 1800; // 100 seconds before child3 ends
        let node = graph.nodes.get_mut("child3").unwrap();
        node.yes_price = Some(0.40); // Underpriced given BTC at 71500 > strike 71430

        let violations = engine.check_late_phase_anomalies(&graph, now);

        // Should detect the late-phase anomaly
        assert!(!violations.is_empty());
    }

    #[test]
    fn test_check_price_consistency() {
        let engine = ConstraintEngine::new(Default::default());
        let mut graph = create_test_graph();

        // Set up a mispriced node
        let node = graph.nodes.get_mut("child3").unwrap();
        node.yes_price = Some(0.30); // Way underpriced
        node.no_price = Some(0.70);

        let now = 1700;
        let violations = engine.check_price_consistency(&graph, now);

        // Should detect price inconsistency
        assert!(!violations.is_empty());
    }

    #[test]
    fn test_expected_value() {
        // Positive EV
        let ev = ConstraintEngine::expected_value(0.60, 0.50, 1.0);
        assert!(ev > 0.0);

        // Negative EV
        let ev = ConstraintEngine::expected_value(0.40, 0.50, 1.0);
        assert!(ev < 0.0);

        // Zero EV
        let ev = ConstraintEngine::expected_value(0.50, 0.50, 1.0);
        assert!((ev - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_kelly_fraction() {
        // Positive edge
        let kelly = ConstraintEngine::kelly_fraction(0.60, 0.50);
        assert!(kelly > 0.0);

        // Negative edge
        let kelly = ConstraintEngine::kelly_fraction(0.40, 0.50);
        assert_eq!(kelly, 0.0);

        // Even odds, fair coin
        let kelly = ConstraintEngine::kelly_fraction(0.50, 0.50);
        assert_eq!(kelly, 0.0);
    }

    #[test]
    fn test_is_spread_acceptable() {
        let engine = ConstraintEngine::new(Default::default());

        // Tight spread
        assert!(engine.is_spread_acceptable(0.49, 0.51, 0.49, 0.51));

        // Wide spread
        assert!(!engine.is_spread_acceptable(0.45, 0.55, 0.45, 0.55));

        // Broken book
        assert!(!engine.is_spread_acceptable(0.50, 0.60, 0.50, 0.60));
    }

    #[test]
    fn test_is_market_broken() {
        let engine = ConstraintEngine::new(Default::default());

        // Normal market
        assert!(!engine.is_market_broken(0.49, 0.51, 0.49, 0.51));

        // Very wide spread
        assert!(engine.is_market_broken(0.30, 0.70, 0.30, 0.70));

        // Broken complement
        assert!(engine.is_market_broken(0.60, 0.65, 0.60, 0.65));
    }
}
