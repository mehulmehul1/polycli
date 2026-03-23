//! Hawkes Flow Excitation Strategy Engine
//!
//! A fundamentally new strategy based on order flow self/cross-excitation dynamics.
//! Uses Hawkes process intensities to detect asymmetric buy/sell pressure before
//! it manifests in price, combined with a VPIN-like toxicity gate.
//!
//! Key insight from literature:
//! - Nittur & Jain (2025): Hawkes SOE kernel best for OFI forecasting
//! - Busetto & Formentin (2023): Hawkes+COE outperforms benchmarks on crypto LOB
//! - Kitvanitphasu et al. (2026): VPIN significantly predicts BTC price jumps
//! - Elomari-Kessab et al. (2024): Microstructure modes via PCA on flow/returns
//!
//! Novel contribution: None of the existing strategies (Heuristic/FairValue/TemporalArb)
//! use order flow excitation dynamics as a signal source.

use super::{
    Confidence, Direction, EntryReason, ExitReason, Observation, SignalSource, StrategyDecision,
    StrategyEngine,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Configuration for the Hawkes Flow strategy
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HawkesFlowConfig {
    /// Decay rate for exponential Hawkes kernel (higher = faster decay)
    pub kernel_decay: f64,
    /// Minimum excitation asymmetry index to trigger entry (0-1)
    pub min_heai: f64,
    /// Maximum excitation asymmetry for confidence capping
    pub max_heai: f64,
    /// VPIN toxicity threshold — only trade when VPIN > this
    pub vpin_threshold: f64,
    /// Window size for VPIN calculation (number of price updates)
    pub vpin_window: usize,
    /// Minimum time remaining to enter (seconds)
    pub min_time_remaining: i64,
    /// Price bounds to avoid extremes
    pub max_entry_prob: f64,
    pub min_entry_prob: f64,
    /// Minimum Bollinger width to ensure volatility
    pub min_bb_width: f64,
    /// Cooldown after exit (observations)
    pub cooldown_observations: usize,
    /// Take profit threshold (positive, e.g. 0.30 = 30%)
    pub base_tp_pct: f64,
    /// Stop loss threshold (negative, e.g. -0.10 = -10%)
    pub base_sl_pct: f64,
}

impl Default for HawkesFlowConfig {
    fn default() -> Self {
        Self {
            kernel_decay: 0.5,      // 500ms half-life
            min_heai: 0.06,         // 6% asymmetry — fewer but higher quality
            max_heai: 0.60,         // 60% for max confidence
            vpin_threshold: 0.05,   // VPIN > 0.05 = flow is present
            vpin_window: 50,        // 50 price updates for VPIN
            min_time_remaining: 45, // 45 seconds minimum
            max_entry_prob: 0.88,
            min_entry_prob: 0.15,
            min_bb_width: 0.0,        // spread proxy — don't gate on this
            cooldown_observations: 3, // 3 observations after exit
            base_tp_pct: 0.30,        // 30% take profit — prediction market winners go to $1
            base_sl_pct: -0.10,       // 10% stop loss — wider for noisy PM prices
        }
    }
}

/// A single order flow event (buy or sell inference from price movement)
#[derive(Debug, Clone, Copy)]
struct FlowEvent {
    timestamp: i64,
    is_buy: bool,   // inferred from price movement direction
    magnitude: f64, // absolute price change
}

/// Hawkes intensity estimator with exponential kernel
#[derive(Debug, Clone)]
struct HawkesEstimator {
    /// Base intensity (background rate)
    mu: f64,
    /// Excitation coefficient
    alpha: f64,
    /// Decay rate
    beta: f64,
    /// History of events
    events: VecDeque<FlowEvent>,
    /// Current intensity value
    current_intensity: f64,
    /// Last update timestamp
    last_ts: Option<i64>,
}

impl HawkesEstimator {
    fn new(alpha: f64, beta: f64) -> Self {
        Self {
            mu: 0.1,
            alpha,
            beta,
            events: VecDeque::with_capacity(200),
            current_intensity: 0.1,
            last_ts: None,
        }
    }

    /// Update intensity with a new event
    fn update(&mut self, event: FlowEvent) {
        // Decay existing intensity since last update
        if let Some(last) = self.last_ts {
            let dt = (event.timestamp - last) as f64 / 1000.0; // ms to seconds
            if dt > 0.0 {
                self.current_intensity =
                    self.mu + (self.current_intensity - self.mu) * (-self.beta * dt).exp();
            }
        }

        // Add excitation from new event
        self.current_intensity += self.alpha * event.magnitude;

        self.events.push_back(event);
        self.last_ts = Some(event.timestamp);

        // Keep only recent events (last 60 seconds)
        let cutoff = event.timestamp - 60;
        while self.events.front().map_or(false, |e| e.timestamp < cutoff) {
            self.events.pop_front();
        }
    }

    /// Get current intensity
    fn intensity(&self) -> f64 {
        self.current_intensity
    }

    /// Decay intensity to a future timestamp
    fn intensity_at(&self, future_ts: i64) -> f64 {
        if let Some(last) = self.last_ts {
            let dt = (future_ts - last) as f64 / 1000.0; // ms to seconds
            if dt > 0.0 {
                return self.mu + (self.current_intensity - self.mu) * (-self.beta * dt).exp();
            }
        }
        self.current_intensity
    }

    fn reset(&mut self) {
        self.events.clear();
        self.current_intensity = self.mu;
        self.last_ts = None;
    }
}

/// Volume-synchronized Probability of Informed Trading (VPIN) estimator
#[derive(Debug, Clone)]
struct VpinEstimator {
    /// Bucket of trade volumes
    bucket_buy_volume: f64,
    bucket_sell_volume: f64,
    /// Completed bucket imbalances
    imbalances: VecDeque<f64>,
    /// Number of buckets for VPIN calculation
    window: usize,
    /// Average bucket size (volume per bucket)
    avg_bucket_volume: f64,
    /// Current bucket fill
    current_bucket_volume: f64,
}

impl VpinEstimator {
    fn new(window: usize) -> Self {
        Self {
            bucket_buy_volume: 0.0,
            bucket_sell_volume: 0.0,
            imbalances: VecDeque::with_capacity(window + 1),
            window,
            avg_bucket_volume: 0.0,
            current_bucket_volume: 0.0,
        }
    }

    /// Add a trade and potentially complete a bucket
    fn add_trade(&mut self, is_buy: bool, volume: f64) {
        if is_buy {
            self.bucket_buy_volume += volume;
        } else {
            self.bucket_sell_volume += volume;
        }
        self.current_bucket_volume += volume;

        // Check if bucket is complete (using average or initial estimate)
        let threshold = if self.avg_bucket_volume > 0.0 {
            self.avg_bucket_volume
        } else {
            10.0 // initial bucket size
        };

        if self.current_bucket_volume >= threshold {
            self.complete_bucket();
        }
    }

    fn complete_bucket(&mut self) {
        let total = self.bucket_buy_volume + self.bucket_sell_volume;
        if total > 0.0 {
            let imbalance = (self.bucket_buy_volume - self.bucket_sell_volume).abs() / total;
            self.imbalances.push_back(imbalance);
            if self.imbalances.len() > self.window {
                self.imbalances.pop_front();
            }
        }

        // Update average bucket volume (exponential moving average)
        if self.avg_bucket_volume == 0.0 {
            self.avg_bucket_volume = self.current_bucket_volume;
        } else {
            self.avg_bucket_volume =
                0.9 * self.avg_bucket_volume + 0.1 * self.current_bucket_volume;
        }

        // Reset bucket
        self.bucket_buy_volume = 0.0;
        self.bucket_sell_volume = 0.0;
        self.current_bucket_volume = 0.0;
    }

    /// Compute current VPIN
    fn vpin(&self) -> f64 {
        if self.imbalances.is_empty() {
            return 0.5; // neutral
        }
        let sum: f64 = self.imbalances.iter().sum();
        sum / self.imbalances.len() as f64
    }

    fn reset(&mut self) {
        self.bucket_buy_volume = 0.0;
        self.bucket_sell_volume = 0.0;
        self.imbalances.clear();
        self.avg_bucket_volume = 0.0;
        self.current_bucket_volume = 0.0;
    }
}

/// Hawkes Excitation Asymmetry Index (HEAI)
/// The core novel signal: measures imbalance between buy-side and sell-side
/// self-excitation intensities from Hawkes process estimation.
fn compute_heai(buy_intensity: f64, sell_intensity: f64) -> f64 {
    let total = buy_intensity + sell_intensity;
    if total < 1e-10 {
        return 0.0;
    }
    (buy_intensity - sell_intensity) / total
}

/// Hawkes Flow Excitation Engine
///
/// Strategy logic:
/// 1. Maintain two Hawkes estimators (buy-side, sell-side)
/// 2. On each observation, infer trade direction from price movement
/// 3. Update both estimators and compute HEAI
/// 4. Compute VPIN from volume imbalance
/// 5. Entry when |HEAI| > threshold AND VPIN > toxicity_gate AND price in range
pub struct HawkesFlowEngine {
    config: HawkesFlowConfig,
    buy_hawkes: HawkesEstimator,
    sell_hawkes: HawkesEstimator,
    vpin: VpinEstimator,
    prev_mid: Option<f64>,
    prev_ts: Option<i64>,
    active_position: bool,
    cooldown_counter: usize,
    /// Track entry price for exit decisions
    entry_price: Option<f64>,
    entry_direction: Option<Direction>,
    /// Rolling HEAI history for momentum detection
    heai_history: VecDeque<f64>,
}

impl HawkesFlowEngine {
    pub fn new() -> Self {
        Self::with_config(HawkesFlowConfig::default())
    }

    pub fn with_config(config: HawkesFlowConfig) -> Self {
        let kernel_alpha = 0.3;
        let kernel_beta = config.kernel_decay;
        Self {
            config,
            buy_hawkes: HawkesEstimator::new(kernel_alpha, kernel_beta),
            sell_hawkes: HawkesEstimator::new(kernel_alpha, kernel_beta),
            vpin: VpinEstimator::new(50),
            prev_mid: None,
            prev_ts: None,
            active_position: false,
            cooldown_counter: 0,
            entry_price: None,
            entry_direction: None,
            heai_history: VecDeque::with_capacity(20),
        }
    }

    /// Infer trade direction and magnitude from price movement
    fn infer_flow_event(&self, obs: &Observation) -> Option<FlowEvent> {
        let mid = obs.yes_mid;
        let ts = obs.ts;

        if let (Some(prev_mid), Some(prev_ts)) = (self.prev_mid, self.prev_ts) {
            let price_change = mid - prev_mid;
            let dt = ts - prev_ts;
            if dt <= 0 {
                return None;
            }

            let magnitude = price_change.abs();
            if magnitude < 1e-6 {
                return None;
            }

            Some(FlowEvent {
                timestamp: ts,
                is_buy: price_change > 0.0,
                magnitude,
            })
        } else {
            None
        }
    }

    /// Estimate volume from book_sum (proxy for available liquidity)
    fn estimate_volume(&self, obs: &Observation) -> f64 {
        obs.book_sum.max(0.1)
    }

    fn check_entry(
        &self,
        obs: &Observation,
        heai: f64,
        vpin_val: f64,
    ) -> Option<(Direction, EntryReason)> {
        let p = obs.yes_mid;

        // Price range filter
        if p > self.config.max_entry_prob || p < self.config.min_entry_prob {
            return None;
        }

        // VPIN toxicity gate
        if vpin_val < self.config.vpin_threshold {
            return None;
        }

        // Bollinger width filter (need volatility)
        let bb_width = obs.indicator_5s.bb_width.unwrap_or(0.0);
        if bb_width < self.config.min_bb_width {
            return None;
        }

        // Time remaining filter
        if obs.time_remaining_s < self.config.min_time_remaining {
            return None;
        }

        // Cooldown check
        if self.cooldown_counter > 0 {
            return None;
        }

        // HEAI threshold check
        let abs_heai = heai.abs();
        if abs_heai < self.config.min_heai {
            return None;
        }

        // HEAI momentum confirmation: recent HEAI should be consistently directional
        if self.heai_history.len() >= 3 {
            let recent: Vec<f64> = self.heai_history.iter().rev().take(3).cloned().collect();
            let all_same_sign = recent.iter().all(|&h| h * heai > 0.0);
            if !all_same_sign {
                return None;
            }
        }

        // Compute confidence from HEAI magnitude and VPIN
        let heai_confidence = (abs_heai / self.config.max_heai).min(1.0);
        let vpin_bonus =
            ((vpin_val - self.config.vpin_threshold) / (1.0 - self.config.vpin_threshold)).min(1.0);
        let confidence_val = (heai_confidence * 0.7 + vpin_bonus * 0.3).min(1.0);

        let direction = if heai > 0.0 {
            Direction::Yes
        } else {
            Direction::No
        };

        let detail = format!(
            "HawkesFlow: HEAI={:.3} VPIN={:.3} conf={:.3} buy_λ={:.4} sell_λ={:.4}",
            heai,
            vpin_val,
            confidence_val,
            self.buy_hawkes.intensity(),
            self.sell_hawkes.intensity(),
        );

        Some((
            direction,
            EntryReason {
                source: SignalSource::HawkesFlow,
                confidence: Confidence::new(confidence_val),
                detail,
                fair_value_edge: None,
                qlib_score: None,
            },
        ))
    }

    fn check_exit(&self, obs: &Observation, heai: f64) -> Option<ExitReason> {
        if !self.active_position {
            return None;
        }

        let p = obs.yes_mid;

        // Time-based exit
        if obs.time_remaining_s < 15 {
            return Some(ExitReason::TimeExpiry {
                seconds_remaining: obs.time_remaining_s,
            });
        }

        // Take profit: price moved significantly in our favor
        if let (Some(entry_price), Some(dir)) = (self.entry_price, self.entry_direction) {
            let move_in_favor = match dir {
                Direction::Yes => p - entry_price,
                Direction::No => entry_price - p,
            };
            if move_in_favor > self.config.base_tp_pct {
                return Some(ExitReason::TakeProfit {
                    pnl_pct: move_in_favor * 100.0,
                });
            }
            // Stop loss
            if move_in_favor < self.config.base_sl_pct {
                return Some(ExitReason::StopLoss {
                    pnl_pct: move_in_favor * 100.0,
                });
            }
        }

        // HEAI reversal exit
        if let Some(dir) = self.entry_direction {
            let heai_against = match dir {
                Direction::Yes => heai < -0.10,
                Direction::No => heai > 0.10,
            };
            if heai_against {
                return Some(ExitReason::MomentumReversal);
            }
        }

        // Extreme probability exit
        if p > 0.95 || p < 0.05 {
            return Some(ExitReason::RiskGate {
                reason: format!("Price at extreme: {:.3}", p),
            });
        }

        None
    }
}

impl StrategyEngine for HawkesFlowEngine {
    fn decide(&mut self, obs: &Observation) -> StrategyDecision {
        // Infer flow event from price movement
        if let Some(event) = self.infer_flow_event(obs) {
            // Update Hawkes estimators
            if event.is_buy {
                self.buy_hawkes.update(event);
            } else {
                self.sell_hawkes.update(event);
            }

            // Update VPIN
            let volume = self.estimate_volume(obs);
            self.vpin.add_trade(event.is_buy, volume);
        }

        // Compute HEAI
        let buy_intensity = self.buy_hawkes.intensity();
        let sell_intensity = self.sell_hawkes.intensity();
        let heai = compute_heai(buy_intensity, sell_intensity);
        let vpin_val = self.vpin.vpin();

        // Track HEAI history
        self.heai_history.push_back(heai);
        if self.heai_history.len() > 20 {
            self.heai_history.pop_front();
        }

        // Decrement cooldown
        if self.cooldown_counter > 0 {
            self.cooldown_counter -= 1;
        }

        // Check exit first if we have a position
        if self.active_position {
            if let Some(exit_reason) = self.check_exit(obs, heai) {
                self.active_position = false;
                self.cooldown_counter = self.config.cooldown_observations;
                self.entry_price = None;
                self.entry_direction = None;
                return StrategyDecision::Exit {
                    position_id: String::new(),
                    reason: exit_reason,
                };
            }
        }

        // Check entry
        if !self.active_position {
            if let Some((direction, reason)) = self.check_entry(obs, heai, vpin_val) {
                self.active_position = true;
                self.entry_price = Some(obs.yes_mid);
                self.entry_direction = Some(direction);
                return StrategyDecision::Enter { direction, reason };
            }
        }

        // Update prev state
        self.prev_mid = Some(obs.yes_mid);
        self.prev_ts = Some(obs.ts);

        StrategyDecision::Hold
    }

    fn reset(&mut self) {
        self.buy_hawkes.reset();
        self.sell_hawkes.reset();
        self.vpin.reset();
        self.prev_mid = None;
        self.prev_ts = None;
        self.active_position = false;
        self.cooldown_counter = 0;
        self.entry_price = None;
        self.entry_direction = None;
        self.heai_history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::indicators::IndicatorState;

    fn make_obs(ts: i64, mid: f64, time_remaining: i64) -> Observation {
        Observation {
            ts,
            condition_id: "test".to_string(),
            market_slug: "btc-up-down".to_string(),
            yes_bid: mid - 0.01,
            yes_ask: mid + 0.01,
            no_bid: 1.0 - mid - 0.01,
            no_ask: 1.0 - mid + 0.01,
            yes_mid: mid,
            no_mid: 1.0 - mid,
            book_sum: 50.0,
            time_remaining_s: time_remaining,
            indicator_5s: IndicatorState {
                bb_width: Some(0.20),
                ..Default::default()
            },
            indicator_1m: IndicatorState::default(),
            fair_value_prob: None,
            qlib_score: None,
        }
    }

    #[test]
    fn engine_starts_in_hold() {
        let mut engine = HawkesFlowEngine::new();
        let obs = make_obs(1000, 0.50, 300);
        let decision = engine.decide(&obs);
        assert!(matches!(decision, StrategyDecision::Hold));
    }

    #[test]
    fn engine_builds_heai_with_directional_flow() {
        let mut engine = HawkesFlowEngine::with_config(HawkesFlowConfig {
            min_heai: 0.10,
            vpin_threshold: 0.15,
            ..Default::default()
        });

        // Simulate sustained buy pressure (price rising)
        let mut ts = 1000;
        let mut price = 0.50;
        for _ in 0..30 {
            price += 0.003; // consistent upward movement
            ts += 1000;
            let _ = engine.decide(&make_obs(ts, price, 300));
        }

        // After sustained buy pressure, HEAI should be positive
        let buy_intensity = engine.buy_hawkes.intensity();
        let sell_intensity = engine.sell_hawkes.intensity();
        assert!(
            buy_intensity > sell_intensity,
            "Buy intensity ({}) should exceed sell intensity ({})",
            buy_intensity,
            sell_intensity
        );
    }

    #[test]
    fn heai_computation() {
        let heai = compute_heai(0.8, 0.2);
        assert!((heai - 0.6).abs() < 1e-6);

        let heai_balanced = compute_heai(0.5, 0.5);
        assert!(heai_balanced.abs() < 1e-6);

        let heai_sell = compute_heai(0.1, 0.9);
        assert!((heai_sell - (-0.8)).abs() < 1e-6);
    }

    #[test]
    fn vpin_estimator_basic() {
        let mut vpin = VpinEstimator::new(5);
        // Add mostly buy trades
        for _ in 0..100 {
            vpin.add_trade(true, 1.0);
        }
        // Force bucket completion
        vpin.complete_bucket();
        assert!(vpin.vpin() > 0.0);
    }
}
