//! Multi-Scale Temporal Arbitrage Engine
//!
//! This module implements a strategy that exploits information flow across
//! Polymarket's nested timeframes: 5min → 15min → 1hour → 4hour.
//!
//! The key insight is that each market has a **resetting strike** (the open
//! of its own interval), creating a chained/path-dependent structure that
//! generates legitimate conditional arbitrage edges.

use crate::bot::strategy::constraint_engine::ConstraintEngine;
use crate::bot::strategy::graph_builder::TemporalGraphBuilder;
use crate::bot::strategy::probability_engine::{
    ArbitrageAction, ArbitrageActionReason, ProbabilityEngine, ViolationType,
};
use crate::bot::strategy::{
    Confidence, Direction, EntryReason, Observation, SignalSource, StrategyDecision,
    StrategyEngine, TemporalGraph,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, SystemTime};

/// Timeframe enumeration for temporal markets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    /// 5 minutes
    M5,
    /// 15 minutes
    M15,
    /// 1 hour
    H1,
    /// 4 hours
    H4,
}

impl Timeframe {
    /// Returns the duration of this timeframe in seconds
    pub fn duration_secs(&self) -> i64 {
        match self {
            Timeframe::M5 => 300,
            Timeframe::M15 => 900,
            Timeframe::H1 => 3600,
            Timeframe::H4 => 14400,
        }
    }

    /// Returns the volatility multiplier for √time scaling
    pub fn vol_multiplier(&self) -> f64 {
        match self {
            Timeframe::M5 => 1.0,
            Timeframe::M15 => (3.0_f64).sqrt(),
            Timeframe::H1 => (12.0_f64).sqrt(),
            Timeframe::H4 => (48.0_f64).sqrt(),
        }
    }

    /// Returns the parent timeframe (next longer duration)
    pub fn parent(&self) -> Option<Timeframe> {
        match self {
            Timeframe::M5 => Some(Timeframe::M15),
            Timeframe::M15 => Some(Timeframe::H1),
            Timeframe::H1 => Some(Timeframe::H4),
            Timeframe::H4 => None,
        }
    }

    /// Returns the child timeframe (next shorter duration)
    pub fn child(&self) -> Option<Timeframe> {
        match self {
            Timeframe::M5 => None,
            Timeframe::M15 => Some(Timeframe::M5),
            Timeframe::H1 => Some(Timeframe::M15),
            Timeframe::H4 => Some(Timeframe::H1),
        }
    }

    /// Returns the number of child intervals that fit in this parent
    pub fn child_count(&self) -> Option<usize> {
        match self {
            Timeframe::M5 => None,
            Timeframe::M15 => Some(3), // 3 x 5min
            Timeframe::H1 => Some(4),  // 4 x 15min
            Timeframe::H4 => Some(4),  // 4 x 1hour
        }
    }
}

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Timeframe::M5 => write!(f, "5m"),
            Timeframe::M15 => write!(f, "15m"),
            Timeframe::H1 => write!(f, "1h"),
            Timeframe::H4 => write!(f, "4h"),
        }
    }
}

impl TryFrom<&str> for Timeframe {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "5m" | "5min" | "5 min" | "5-minute" => Ok(Timeframe::M5),
            "15m" | "15min" | "15 min" | "15-minute" => Ok(Timeframe::M15),
            "1h" | "1hour" | "1 hour" | "1-hour" => Ok(Timeframe::H1),
            "4h" | "4hour" | "4 hour" | "4-hour" => Ok(Timeframe::H4),
            _ => Err(format!("Unknown timeframe: {}", s)),
        }
    }
}

/// A single node in the temporal market graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalNode {
    /// Unique identifier (condition_id or market slug)
    pub condition_id: String,
    /// Timeframe of this market
    pub timeframe: Timeframe,
    /// Strike price for this market (resets per interval)
    pub strike_price: f64,
    /// Market start time (Unix timestamp)
    pub start_time: i64,
    /// Market end time (Unix timestamp)
    pub end_time: i64,
    /// Parent node ID (if any)
    pub parent: Option<String>,
    /// Child node IDs
    pub children: Vec<String>,
    /// Current YES price
    pub yes_price: Option<f64>,
    /// Current NO price
    pub no_price: Option<f64>,
    /// Resolved outcome (if known)
    pub resolved_outcome: Option<bool>,
    /// Actual closing price when resolved
    pub close_price: Option<f64>,
}

impl TemporalNode {
    /// Check if this node is currently active (not resolved)
    pub fn is_active(&self, now: i64) -> bool {
        now < self.end_time && self.resolved_outcome.is_none()
    }

    /// Check if this node has resolved
    pub fn is_resolved(&self, now: i64) -> bool {
        self.resolved_outcome.is_some() || now >= self.end_time
    }

    /// Get remaining time in seconds
    pub fn time_remaining(&self, now: i64) -> i64 {
        (self.end_time - now).max(0)
    }

    /// Get elapsed time in seconds
    pub fn time_elapsed(&self, now: i64) -> i64 {
        (now - self.start_time).max(0)
    }

    /// Get progress as a fraction (0.0 to 1.0)
    pub fn progress(&self, now: i64) -> f64 {
        let total = self.end_time - self.start_time;
        if total <= 0 {
            return 1.0;
        }
        (self.time_elapsed(now) as f64 / total as f64).min(1.0)
    }

    /// Update price from observation
    pub fn update_price(&mut self, yes_price: f64, no_price: f64) {
        self.yes_price = Some(yes_price);
        self.no_price = Some(no_price);
    }

    /// Mark as resolved with outcome
    pub fn mark_resolved(&mut self, outcome: bool, close_price: f64) {
        self.resolved_outcome = Some(outcome);
        self.close_price = Some(close_price);
    }
}

/// Price state tracking for the temporal graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceState {
    /// Current BTC price
    pub current_price: f64,
    /// Last update timestamp
    pub last_update: i64,
    /// Realized volatility estimate
    pub realized_vol: f64,
    /// Historical price path
    pub price_path: VecDeque<(i64, f64)>,
}

impl Default for PriceState {
    fn default() -> Self {
        Self {
            current_price: 0.0,
            last_update: 0,
            realized_vol: 500.0, // Base BTC vol ~$500/day
            price_path: VecDeque::with_capacity(100),
        }
    }
}

impl PriceState {
    /// Update with new price
    pub fn update(&mut self, price: f64, ts: i64) {
        if self.current_price > 0.0 {
            let ret = (price / self.current_price).ln();
            self.price_path.push_back((ts, ret));
            if self.price_path.len() > 100 {
                self.price_path.pop_front();
            }

            // Update realized vol
            if self.price_path.len() >= 10 {
                let mean: f64 = self.price_path.iter().map(|(_, r)| r).sum::<f64>()
                    / self.price_path.len() as f64;
                let variance: f64 = self
                    .price_path
                    .iter()
                    .map(|(_, r)| (r - mean).powi(2))
                    .sum::<f64>()
                    / (self.price_path.len() as f64 - 1.0);
                self.realized_vol = (variance.sqrt() * price * 100.0).max(200.0).min(2000.0);
            }
        }
        self.current_price = price;
        self.last_update = ts;
    }

    /// Get age of last update in seconds
    pub fn age(&self, now: i64) -> i64 {
        (now - self.last_update).max(0)
    }

    /// Check if price is stale
    pub fn is_stale(&self, now: i64, max_age: i64) -> bool {
        self.age(now) > max_age
    }
}

/// Volatility estimator for the temporal graph
#[derive(Debug, Clone)]
pub struct VolatilityEstimator {
    /// Base volatility for 5min timeframe (USD price move)
    pub base_vol_5m: f64,
    /// Recent returns for EWMA calculation
    pub recent_returns: VecDeque<f64>,
    /// Lambda for EWMA (decay factor)
    pub lambda: f64,
    /// Maximum window size
    max_window: usize,
}

impl Default for VolatilityEstimator {
    fn default() -> Self {
        Self {
            base_vol_5m: 250.0, // ~$250 move over 5min for BTC
            recent_returns: VecDeque::with_capacity(20),
            lambda: 0.94,
            max_window: 20,
        }
    }
}

impl VolatilityEstimator {
    /// Create with custom parameters
    pub fn new(base_vol_5m: f64, lambda: f64) -> Self {
        Self {
            base_vol_5m,
            recent_returns: VecDeque::with_capacity(20),
            lambda,
            max_window: 20,
        }
    }

    /// Update with new price observation
    pub fn update(&mut self, price: f64) {
        if let Some(&last_price) = self.recent_returns.back() {
            let ret = (price - last_price) / last_price;
            self.recent_returns.push_back(ret);
            if self.recent_returns.len() > self.max_window {
                self.recent_returns.pop_front();
            }
        } else {
            self.recent_returns.push_back(price);
        }
    }

    /// Get volatility per second
    pub fn vol_per_second(&self) -> f64 {
        self.base_vol_5m / 300.0
    }

    /// Get realized volatility using EWMA
    pub fn realized_volatility(&self) -> f64 {
        if self.recent_returns.len() < 2 {
            return self.base_vol_5m;
        }

        // Calculate EWMA volatility
        let mut ewma_var = 0.0;
        let mut weight = 1.0;

        for i in 1..self.recent_returns.len() {
            let ret = self.recent_returns[i] - self.recent_returns[i - 1];
            ewma_var = self.lambda * ewma_var + (1.0 - self.lambda) * ret * ret;
            weight *= self.lambda;
        }

        (ewma_var.sqrt() * 100.0).max(self.base_vol_5m * 0.5)
    }

    /// Get volatility for a specific timeframe
    pub fn vol_for_timeframe(&self, timeframe: Timeframe) -> f64 {
        let vol_per_sec = self.vol_per_second();
        let duration = timeframe.duration_secs() as f64;
        vol_per_sec * duration.sqrt()
    }
}

/// Configuration for the temporal arbitrage engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalArbitrageConfig {
    /// Minimum edge to enter a trade (probability units)
    pub min_edge: f64,
    /// Maximum spread allowed
    pub max_spread: f64,
    /// Enable chained conditional arbitrage
    pub enable_chained_conditional: bool,
    /// Enable late-phase anomaly detection
    pub enable_late_phase: bool,
    /// Enable price consistency checks
    pub enable_price_consistency: bool,
    /// Base volatility for 5min timeframe
    pub base_volatility_5m: f64,
    /// Minimum confidence threshold
    pub min_confidence: f64,
    /// Maximum number of concurrent positions
    pub max_concurrent_positions: usize,
}

impl Default for TemporalArbitrageConfig {
    fn default() -> Self {
        Self {
            min_edge: 0.05,
            max_spread: 0.03,
            enable_chained_conditional: true,
            enable_late_phase: true,
            enable_price_consistency: true,
            base_volatility_5m: 250.0,
            min_confidence: 0.5,
            max_concurrent_positions: 3,
        }
    }
}

/// Position tracking for temporal arbitrage
#[derive(Debug, Clone)]
pub struct TemporalPosition {
    /// Condition ID being traded
    pub condition_id: String,
    /// Direction (Yes/No)
    pub direction: Direction,
    /// Entry price
    pub entry_price: f64,
    /// Entry timestamp
    pub entry_ts: i64,
    /// Position size (USD)
    pub size_usd: f64,
    /// Expected edge at entry
    pub expected_edge: f64,
    /// Original violation type
    pub violation_type: ViolationType,
}

impl TemporalPosition {
    /// Calculate unrealized PnL
    pub fn unrealized_pnl(&self, current_price: f64) -> f64 {
        match self.direction {
            Direction::Yes => (current_price - self.entry_price) / self.entry_price,
            Direction::No => (self.entry_price - current_price) / self.entry_price,
        }
    }

    /// Check if position should be exited
    pub fn should_exit(&self, current_price: f64, time_remaining: i64) -> bool {
        let pnl = self.unrealized_pnl(current_price);

        // Exit on profit
        if pnl > 0.1 {
            return true;
        }

        // Exit on loss
        if pnl < -0.05 {
            return true;
        }

        // Exit near expiry
        if time_remaining < 60 {
            return true;
        }

        false
    }
}

/// The main temporal arbitrage engine
pub struct TemporalArbitrageEngine {
    /// Engine configuration
    config: TemporalArbitrageConfig,
    /// Temporal graph of markets
    graph: Option<TemporalGraph>,
    /// Constraint engine for detecting violations
    constraint_engine: ConstraintEngine,
    /// Graph builder
    graph_builder: TemporalGraphBuilder,
    /// Active positions
    positions: HashMap<String, TemporalPosition>,
    /// Last update timestamp
    last_update: i64,
    /// Statistics
    stats: EngineStats,
}

/// Engine statistics
#[derive(Debug, Clone, Default)]
pub struct EngineStats {
    pub violations_detected: usize,
    pub entries_taken: usize,
    pub entries_blocked: usize,
    pub total_edge: f64,
}

impl TemporalArbitrageEngine {
    /// Create a new temporal arbitrage engine
    pub fn new(config: TemporalArbitrageConfig) -> Self {
        Self {
            constraint_engine: ConstraintEngine::new(config.clone()),
            graph_builder: TemporalGraphBuilder::new(),
            graph: None,
            positions: HashMap::new(),
            last_update: 0,
            stats: EngineStats::default(),
            config,
        }
    }

    /// Create with default configuration
    pub fn default_config() -> Self {
        Self::new(TemporalArbitrageConfig::default())
    }

    /// Initialize the graph with market data
    pub fn initialize_graph(&mut self, markets: Vec<TemporalNode>, btc_price: f64, now: i64) {
        self.graph = Some(self.graph_builder.build_from_nodes(markets, btc_price, now));
        self.last_update = now;
    }

    /// Get current graph state
    pub fn graph(&self) -> Option<&TemporalGraph> {
        self.graph.as_ref()
    }

    /// Get active positions
    pub fn positions(&self) -> &HashMap<String, TemporalPosition> {
        &self.positions
    }

    /// Get engine statistics
    pub fn stats(&self) -> &EngineStats {
        &self.stats
    }

    /// Get engine config
    pub fn config(&self) -> &TemporalArbitrageConfig {
        &self.config
    }

    /// Update price state
    pub fn update_price(&mut self, btc_price: f64, ts: i64) {
        if let Some(graph) = &mut self.graph {
            graph.price_state.update(btc_price, ts);
        }
        self.last_update = ts;
    }

    /// Find all constraint violations
    fn find_violations(&self, now: i64) -> Vec<ArbitrageAction> {
        let Some(graph) = &self.graph else {
            return Vec::new();
        };

        let mut violations = Vec::new();

        // Check chained conditional (PRIMARY EDGE)
        if self.config.enable_chained_conditional {
            for root in &graph.roots {
                if let Some(v) = self
                    .constraint_engine
                    .check_chained_conditional(graph, root, now)
                {
                    violations.push(v);
                }
            }
        }

        // Late-phase anomalies
        if self.config.enable_late_phase {
            violations.extend(
                self.constraint_engine
                    .check_late_phase_anomalies(graph, now),
            );
        }

        // Price consistency
        if self.config.enable_price_consistency {
            violations.extend(self.constraint_engine.check_price_consistency(graph, now));
        }

        violations
    }

    /// Select the best action from violations
    fn select_best_action(&self, violations: Vec<ArbitrageAction>) -> ArbitrageAction {
        let mut sorted: Vec<_> = violations
            .into_iter()
            .filter(|v| match v {
                ArbitrageAction::EnterSingle { edge, .. } => {
                    *edge >= self.config.min_edge && !self.positions.contains_key(&v.condition_id())
                }
                _ => true,
            })
            .collect();

        sorted.sort_by(|a, b| {
            b.edge()
                .partial_cmp(&a.edge())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.confidence().partial_cmp(&a.confidence()).unwrap())
        });

        sorted.into_iter().next().unwrap_or(ArbitrageAction::Hold)
    }

    /// Remove expired positions
    fn cleanup_positions(&mut self, now: i64) {
        let mut to_remove = Vec::new();
        for (id, pos) in &self.positions {
            if let Some(graph) = &self.graph {
                if let Some(node) = graph.nodes.get(id) {
                    if node.is_resolved(now) || pos.entry_ts + 3600 < now {
                        to_remove.push(id.clone());
                    }
                }
            }
        }
        for id in to_remove {
            self.positions.remove(&id);
        }
    }
}

impl StrategyEngine for TemporalArbitrageEngine {
    fn decide(&mut self, obs: &Observation) -> StrategyDecision {
        let now = obs.ts;

        // Update graph with latest observation
        if let Some(graph) = &mut self.graph {
            if let Some(node) = graph.nodes.get_mut(&obs.condition_id) {
                node.update_price(obs.yes_mid, obs.no_mid);

                // Update price state if we have a fair value or use midpoint
                let price = obs.fair_value_prob.unwrap_or(obs.yes_mid) * 75000.0; // Rough BTC price scaling
                graph.price_state.update(price, now);
            }
        }

        // Clean up expired positions
        self.cleanup_positions(now);

        // Find violations
        let violations = self.find_violations(now);
        self.stats.violations_detected += violations.len();

        if violations.is_empty() {
            return StrategyDecision::Hold;
        }

        // Select best action
        let action = self.select_best_action(violations);

        match action {
            ArbitrageAction::EnterSingle {
                condition_id,
                direction,
                edge,
                reason,
            } => {
                // Check if we can enter
                if self.positions.len() >= self.config.max_concurrent_positions {
                    self.stats.entries_blocked += 1;
                    return StrategyDecision::Block {
                        reason: "Max concurrent positions reached".to_string(),
                    };
                }

                if let Some(graph) = &self.graph {
                    if let Some(node) = graph.nodes.get(&condition_id) {
                        let entry_price = match direction {
                            Direction::Yes => node.yes_price.unwrap_or(obs.yes_mid),
                            Direction::No => node.no_price.unwrap_or(obs.no_mid),
                        };

                        // Create position
                        self.positions.insert(
                            condition_id.clone(),
                            TemporalPosition {
                                condition_id: condition_id.clone(),
                                direction,
                                entry_price,
                                entry_ts: now,
                                size_usd: 10.0, // Default size
                                expected_edge: edge,
                                violation_type: match reason {
                                    ArbitrageActionReason::ChainedConditional(_) => {
                                        ViolationType::ChainedConditional
                                    }
                                    ArbitrageActionReason::LatePhaseAnomaly(_) => {
                                        ViolationType::LatePhaseAnomaly
                                    }
                                    ArbitrageActionReason::PriceConsistency(_) => {
                                        ViolationType::PriceConsistency
                                    }
                                },
                            },
                        );

                        self.stats.entries_taken += 1;
                        self.stats.total_edge += edge;

                        return StrategyDecision::Enter {
                            direction,
                            reason: EntryReason {
                                source: SignalSource::FairValue,
                                confidence: Confidence::new(0.6 + edge * 2.0),
                                detail: reason.to_string(),
                                fair_value_edge: Some(edge),
                                qlib_score: None,
                            },
                        };
                    }
                }
            }
            ArbitrageAction::Hold => {}
        }

        StrategyDecision::Hold
    }

    fn reset(&mut self) {
        self.positions.clear();
        self.stats = EngineStats::default();
        if let Some(graph) = &mut self.graph {
            graph.price_state = PriceState::default();
        }
    }
}

// Helper for ArbitrageAction
impl ArbitrageAction {
    fn condition_id(&self) -> String {
        match self {
            ArbitrageAction::EnterSingle { condition_id, .. } => condition_id.clone(),
            ArbitrageAction::Hold => String::new(),
        }
    }

    fn edge(&self) -> f64 {
        match self {
            ArbitrageAction::EnterSingle { edge, .. } => *edge,
            ArbitrageAction::Hold => 0.0,
        }
    }

    fn confidence(&self) -> f64 {
        match self {
            ArbitrageAction::EnterSingle { reason, .. } => match reason {
                ArbitrageActionReason::ChainedConditional(c)
                | ArbitrageActionReason::LatePhaseAnomaly(c)
                | ArbitrageActionReason::PriceConsistency(c) => *c,
            },
            ArbitrageAction::Hold => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeframe_durations() {
        assert_eq!(Timeframe::M5.duration_secs(), 300);
        assert_eq!(Timeframe::M15.duration_secs(), 900);
        assert_eq!(Timeframe::H1.duration_secs(), 3600);
        assert_eq!(Timeframe::H4.duration_secs(), 14400);
    }

    #[test]
    fn test_timeframe_vol_multiplier() {
        assert!((Timeframe::M5.vol_multiplier() - 1.0).abs() < f64::EPSILON);
        assert!((Timeframe::M15.vol_multiplier() - 3.0_f64.sqrt()).abs() < 1e-10);
    }

    #[test]
    fn test_timeframe_parent_child() {
        assert_eq!(Timeframe::M5.parent(), Some(Timeframe::M15));
        assert_eq!(Timeframe::M15.parent(), Some(Timeframe::H1));
        assert_eq!(Timeframe::H1.parent(), Some(Timeframe::H4));
        assert_eq!(Timeframe::H4.parent(), None);

        assert_eq!(Timeframe::M5.child(), None);
        assert_eq!(Timeframe::M15.child(), Some(Timeframe::M5));
    }

    #[test]
    fn test_timeframe_child_count() {
        assert_eq!(Timeframe::M15.child_count(), Some(3));
        assert_eq!(Timeframe::H1.child_count(), Some(4));
        assert_eq!(Timeframe::H4.child_count(), Some(4));
    }

    #[test]
    fn test_temporal_node_active() {
        let node = TemporalNode {
            condition_id: "test".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 1300,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        assert!(node.is_active(1200));
        assert!(!node.is_active(1400));
    }

    #[test]
    fn test_temporal_node_progress() {
        let node = TemporalNode {
            condition_id: "test".to_string(),
            timeframe: Timeframe::M5,
            strike_price: 71000.0,
            start_time: 1000,
            end_time: 1300,
            parent: None,
            children: Vec::new(),
            yes_price: None,
            no_price: None,
            resolved_outcome: None,
            close_price: None,
        };

        assert!((node.progress(1000) - 0.0).abs() < f64::EPSILON);
        assert!((node.progress(1150) - 0.5).abs() < 0.01);
        assert!((node.progress(1300) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_price_state_update() {
        let mut state = PriceState::default();
        state.update(71000.0, 1000);
        assert_eq!(state.current_price, 71000.0);
        assert_eq!(state.last_update, 1000);
        assert!(!state.is_stale(1050, 100));

        state.update(71100.0, 1100);
        assert_eq!(state.current_price, 71100.0);
        assert_eq!(state.age(1150), 50);
    }

    #[test]
    fn test_volatility_estimator() {
        let estimator = VolatilityEstimator::new(250.0, 0.94);

        assert!((estimator.vol_per_second() - 250.0 / 300.0).abs() < 1e-10);

        let vol_5m = estimator.vol_for_timeframe(Timeframe::M5);
        assert!((vol_5m - 250.0).abs() < 1e-10);

        let vol_15m = estimator.vol_for_timeframe(Timeframe::M15);
        assert!((vol_15m - 250.0 * 3.0_f64.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn test_temporal_position_unrealized_pnl() {
        let position = TemporalPosition {
            condition_id: "test".to_string(),
            direction: Direction::Yes,
            entry_price: 0.50,
            entry_ts: 1000,
            size_usd: 10.0,
            expected_edge: 0.10,
            violation_type: ViolationType::ChainedConditional,
        };

        // 10% profit
        let pnl = position.unrealized_pnl(0.55);
        assert!((pnl - 0.10).abs() < 1e-10);

        // 10% loss
        let pnl = position.unrealized_pnl(0.45);
        assert!((pnl - (-0.10)).abs() < 1e-10);
    }

    #[test]
    fn test_temporal_position_should_exit() {
        let position = TemporalPosition {
            condition_id: "test".to_string(),
            direction: Direction::Yes,
            entry_price: 0.50,
            entry_ts: 1000,
            size_usd: 10.0,
            expected_edge: 0.10,
            violation_type: ViolationType::ChainedConditional,
        };

        // Exit on profit
        assert!(position.should_exit(0.56, 300));

        // Exit on loss
        assert!(position.should_exit(0.47, 300));

        // Exit near expiry
        assert!(position.should_exit(0.51, 30));

        // Hold otherwise
        assert!(!position.should_exit(0.51, 300));
    }
}
