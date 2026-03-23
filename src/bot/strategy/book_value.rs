//! BookValue Strategy Engine
//!
//! Research-driven strategy combining three signals:
//! 1. Book imbalance (OBI-inspired) — spread asymmetry between YES and NO
//! 2. Volume-Adjusted Mid Price deviation — fair value vs traded price
//! 3. Mean reversion — fading recent extremes
//!
//! Based on: Cont et al. (OBI ~65% R²), Hasbrouck (VAMP > mid), Becker (taker toxicity -1.12%)

use super::types::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Configuration for the BookValue strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookValueConfig {
    /// Z-score threshold for entry signal
    pub entry_threshold: f64,
    /// Take profit percentage (0.03 = 3%)
    pub take_profit_pct: f64,
    /// Stop loss percentage (0.05 = 5%)
    pub stop_loss_pct: f64,
    /// Minimum seconds between entries
    pub min_seconds_between_entries: i64,
    /// Minimum time remaining to enter (seconds)
    pub min_time_remaining: i64,
    /// Rolling window size for z-score computation
    pub zscore_window: usize,
    /// Price bounds to avoid extremes
    pub max_entry_prob: f64,
    pub min_entry_prob: f64,
    /// Signal weights (sum should be ~1.0)
    pub w_obi: f64,
    pub w_vamp: f64,
    pub w_reversion: f64,
    /// Minimum book_sum to consider market healthy
    pub min_book_sum: f64,
    pub max_book_sum: f64,
}

impl Default for BookValueConfig {
    fn default() -> Self {
        Self {
            entry_threshold: 1.6,
            take_profit_pct: 0.08,
            stop_loss_pct: 0.06,
            min_seconds_between_entries: 15,
            min_time_remaining: 120,
            zscore_window: 40,
            max_entry_prob: 0.85,
            min_entry_prob: 0.15,
            w_obi: 0.7,
            w_vamp: 0.15,
            w_reversion: 0.15,
            min_book_sum: 0.97,
            max_book_sum: 1.03,
        }
    }
}

/// Rolling z-score tracker
struct ZScoreTracker {
    values: VecDeque<f64>,
    window: usize,
}

impl ZScoreTracker {
    fn new(window: usize) -> Self {
        Self {
            values: VecDeque::with_capacity(window * 2),
            window,
        }
    }

    fn push(&mut self, val: f64) {
        self.values.push_back(val);
        if self.values.len() > self.window * 2 {
            self.values.pop_front();
        }
    }

    fn zscore(&self, val: f64) -> f64 {
        if self.values.len() < 10 {
            return 0.0;
        }
        let n = self.values.len() as f64;
        let mean: f64 = self.values.iter().sum::<f64>() / n;
        let variance: f64 = self.values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let std = variance.sqrt();
        if std < 1e-10 {
            return 0.0;
        }
        (val - mean) / std
    }

    fn reset(&mut self) {
        self.values.clear();
    }
}

/// BookValue strategy engine
pub struct BookValueEngine {
    config: BookValueConfig,
    obi_tracker: ZScoreTracker,
    vamp_tracker: ZScoreTracker,
    reversion_tracker: ZScoreTracker,
    prev_mid: Option<f64>,
    prev_ts: Option<i64>,
    active_position: bool,
    entry_price: Option<f64>,
    entry_direction: Option<Direction>,
    entry_ts: Option<i64>,
    last_exit_ts: Option<i64>,
    peak_price: Option<f64>,
    trailing_active: bool,
}

impl BookValueEngine {
    pub fn new() -> Self {
        Self::with_config(BookValueConfig::default())
    }

    pub fn with_config(config: BookValueConfig) -> Self {
        let window = config.zscore_window;
        Self {
            config,
            obi_tracker: ZScoreTracker::new(window),
            vamp_tracker: ZScoreTracker::new(window),
            reversion_tracker: ZScoreTracker::new(window),
            prev_mid: None,
            prev_ts: None,
            active_position: false,
            entry_price: None,
            entry_direction: None,
            entry_ts: None,
            last_exit_ts: None,
            peak_price: None,
            trailing_active: false,
        }
    }

    /// Compute Book Imbalance (OBI-inspired)
    /// Uses spread asymmetry: wider YES spread = less YES demand = bearish
    fn compute_obi(&self, obs: &super::Observation) -> f64 {
        let yes_spread = obs.yes_ask - obs.yes_bid;
        let no_spread = obs.no_ask - obs.no_bid;

        // Avoid division by zero
        let total = yes_spread + no_spread;
        if total < 1e-10 {
            return 0.0;
        }

        // When YES spread > NO spread: YES is less liquid → NO side has more pressure
        // OBI > 0: NO-heavy (bearish YES), OBI < 0: YES-heavy (bullish YES)
        (no_spread - yes_spread) / total
    }

    /// Compute Volume-Adjusted Mid Price deviation
    /// VAMP weights price by liquidity on the opposite side
    fn compute_vamp_deviation(&self, obs: &super::Observation) -> f64 {
        // Simplified VAMP: weight by inverse spread (tighter spread = more weight)
        let yes_weight = 1.0 / (obs.yes_ask - obs.yes_bid + 0.001);
        let no_weight = 1.0 / (obs.no_ask - obs.no_bid + 0.001);
        let total_weight = yes_weight + no_weight;

        if total_weight < 1e-10 {
            return 0.0;
        }

        // VAMP-implied YES probability
        let vamp_yes = (obs.yes_mid * yes_weight + (1.0 - obs.no_mid) * no_weight) / total_weight;

        // Deviation: positive = market underpriced YES, negative = overpriced YES
        vamp_yes - obs.yes_mid
    }

    /// Compute mean reversion signal
    /// Fades recent price moves, scaled by OBI extremity
    fn compute_reversion(&self, obs: &super::Observation, obi_z: f64) -> f64 {
        let current_mid = obs.yes_mid;
        if let Some(prev) = self.prev_mid {
            let ret = current_mid - prev;
            // Reversion signal: negative return × |OBI| → bet against the move
            -ret * obi_z.abs()
        } else {
            0.0
        }
    }

    fn check_exit(&mut self, obs: &super::Observation) -> Option<ExitReason> {
        if !self.active_position {
            return None;
        }

        let p = obs.yes_mid;
        let dir = match self.entry_direction {
            Some(d) => d,
            None => return None,
        };
        let entry = match self.entry_price {
            Some(e) => e,
            None => return None,
        };

        // Move in our favor (positive = profit)
        let move_in_favor = match dir {
            Direction::Yes => p - entry,
            Direction::No => entry - p,
        };
        let pnl_pct = move_in_favor * 100.0;

        // Time expiry
        if obs.time_remaining_s < 15 {
            return Some(ExitReason::TimeExpiry {
                seconds_remaining: obs.time_remaining_s,
            });
        }

        // Track peak for trailing stop
        let is_new_peak = match dir {
            Direction::Yes => self.peak_price.map_or(true, |peak| p > peak),
            Direction::No => self.peak_price.map_or(true, |peak| p < peak),
        };
        if is_new_peak {
            self.peak_price = Some(p);
        }

        // Activate trailing at +5%
        if !self.trailing_active && move_in_favor >= 0.05 {
            self.trailing_active = true;
        }

        // Trailing stop (5% from peak)
        if self.trailing_active {
            let peak = self.peak_price.unwrap_or(entry);
            let trail_trigger = match dir {
                Direction::Yes => peak * 0.95,
                Direction::No => peak * 1.05,
            };
            let hit_trail = match dir {
                Direction::Yes => p <= trail_trigger,
                Direction::No => p >= trail_trigger,
            };
            if hit_trail {
                return Some(ExitReason::TakeProfit { pnl_pct });
            }
        }

        // Fixed take profit
        if !self.trailing_active && move_in_favor >= self.config.take_profit_pct {
            return Some(ExitReason::TakeProfit { pnl_pct });
        }

        // Stop loss
        if move_in_favor <= -self.config.stop_loss_pct {
            return Some(ExitReason::StopLoss { pnl_pct });
        }

        // Extreme price exit
        if p > 0.95 || p < 0.05 {
            return Some(ExitReason::RiskGate {
                reason: format!("Price at extreme: {:.3}", p),
            });
        }

        None
    }

    fn reset_position_state(&mut self, ts: i64) {
        self.active_position = false;
        self.entry_price = None;
        self.entry_direction = None;
        self.entry_ts = None;
        self.last_exit_ts = Some(ts);
        self.peak_price = None;
        self.trailing_active = false;
    }
}

impl super::StrategyEngine for BookValueEngine {
    fn decide(&mut self, obs: &super::Observation) -> StrategyDecision {
        let p = obs.yes_mid;

        // Compute signals
        let obi = self.compute_obi(obs);
        let vamp_dev = self.compute_vamp_deviation(obs);

        // Z-score normalize
        self.obi_tracker.push(obi);
        self.vamp_tracker.push(vamp_dev);
        let obi_z = self.obi_tracker.zscore(obi);
        let vamp_z = self.vamp_tracker.zscore(vamp_dev);

        // Mean reversion signal
        let reversion = self.compute_reversion(obs, obi_z);
        self.reversion_tracker.push(reversion);
        let reversion_z = self.reversion_tracker.zscore(reversion);

        // Update prev state
        self.prev_mid = Some(p);
        self.prev_ts = Some(obs.ts);

        // Check exit if position active
        if self.active_position {
            if let Some(exit_reason) = self.check_exit(obs) {
                self.reset_position_state(obs.ts);
                return StrategyDecision::Exit {
                    position_id: String::new(),
                    reason: exit_reason,
                };
            }
            return StrategyDecision::Hold;
        }

        // Time-based cooldown
        if let Some(last_exit) = self.last_exit_ts {
            if obs.ts - last_exit < self.config.min_seconds_between_entries {
                return StrategyDecision::Hold;
            }
        }

        // Price range filter
        if p > self.config.max_entry_prob || p < self.config.min_entry_prob {
            return StrategyDecision::Hold;
        }

        // Book health check
        if obs.book_sum < self.config.min_book_sum || obs.book_sum > self.config.max_book_sum {
            return StrategyDecision::Hold;
        }

        // Time remaining check
        if obs.time_remaining_s < self.config.min_time_remaining {
            return StrategyDecision::Hold;
        }

        // Need enough history for reliable z-scores
        if self.obi_tracker.values.len() < 15 {
            return StrategyDecision::Hold;
        }

        // Fused signal
        let signal = self.config.w_obi * obi_z
            + self.config.w_vamp * vamp_z
            + self.config.w_reversion * reversion_z;

        // Entry logic
        if signal > self.config.entry_threshold {
            self.active_position = true;
            self.entry_price = Some(obs.yes_ask); // buy at ask
            self.entry_direction = Some(Direction::Yes);
            self.entry_ts = Some(obs.ts);
            self.peak_price = Some(obs.yes_mid);
            self.trailing_active = false;

            return StrategyDecision::Enter {
                direction: Direction::Yes,
                reason: EntryReason {
                    source: SignalSource::BookInefficiency,
                    confidence: Confidence::new((signal / 3.0).clamp(0.0, 1.0)),
                    detail: format!(
                        "BookValue: OBI={:.2} VAMP={:.2} MR={:.2} S={:.2}",
                        obi_z, vamp_z, reversion_z, signal
                    ),
                    fair_value_edge: Some(vamp_dev),
                    qlib_score: None,
                },
            };
        } else if signal < -self.config.entry_threshold {
            self.active_position = true;
            self.entry_price = Some(obs.no_ask); // buy NO at ask
            self.entry_direction = Some(Direction::No);
            self.entry_ts = Some(obs.ts);
            self.peak_price = Some(obs.yes_mid);
            self.trailing_active = false;

            return StrategyDecision::Enter {
                direction: Direction::No,
                reason: EntryReason {
                    source: SignalSource::BookInefficiency,
                    confidence: Confidence::new((signal.abs() / 3.0).clamp(0.0, 1.0)),
                    detail: format!(
                        "BookValue: OBI={:.2} VAMP={:.2} MR={:.2} S={:.2}",
                        obi_z, vamp_z, reversion_z, signal
                    ),
                    fair_value_edge: Some(vamp_dev),
                    qlib_score: None,
                },
            };
        }

        StrategyDecision::Hold
    }

    fn reset(&mut self) {
        self.obi_tracker.reset();
        self.vamp_tracker.reset();
        self.reversion_tracker.reset();
        self.prev_mid = None;
        self.prev_ts = None;
        self.active_position = false;
        self.entry_price = None;
        self.entry_direction = None;
        self.entry_ts = None;
        self.last_exit_ts = None;
        self.peak_price = None;
        self.trailing_active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::indicators::IndicatorState;

    fn make_obs(ts: i64, yes_mid: f64, time_remaining: i64) -> super::super::Observation {
        let spread = 0.01;
        super::super::Observation {
            ts,
            condition_id: "test".to_string(),
            market_slug: "test".to_string(),
            yes_bid: yes_mid - spread / 2.0,
            yes_ask: yes_mid + spread / 2.0,
            no_bid: 1.0 - yes_mid - spread / 2.0,
            no_ask: 1.0 - yes_mid + spread / 2.0,
            yes_mid,
            no_mid: 1.0 - yes_mid,
            book_sum: 1.0,
            time_remaining_s: time_remaining,
            indicator_5s: IndicatorState::default(),
            indicator_1m: IndicatorState::default(),
            fair_value_prob: None,
            qlib_score: None,
        }
    }

    #[test]
    fn test_obi_symmetric_book() {
        let engine = BookValueEngine::new();
        let obs = make_obs(1000, 0.50, 300);
        let obi = engine.compute_obi(&obs);
        // Symmetric book → OBI = 0
        assert!(obi.abs() < 1e-10, "Symmetric book should give OBI=0");
    }

    #[test]
    fn test_vamp_deviation() {
        let engine = BookValueEngine::new();
        let obs = make_obs(1000, 0.50, 300);
        let vamp = engine.compute_vamp_deviation(&obs);
        // Symmetric book → VAMP ≈ mid → deviation ≈ 0
        assert!(
            vamp.abs() < 0.01,
            "Symmetric book should give small VAMP deviation"
        );
    }

    #[test]
    fn test_no_entry_extreme_prices() {
        let mut engine = BookValueEngine::new();
        // Build up history
        for i in 0..30 {
            let _ = engine.decide(&make_obs(i * 1000, 0.50, 300));
        }
        // Try entry at extreme price
        let obs = make_obs(31000, 0.92, 300);
        let decision = engine.decide(&obs);
        assert!(
            matches!(decision, StrategyDecision::Hold),
            "Should not enter at 0.92"
        );
    }

    #[test]
    fn test_trailing_stop() {
        let mut engine = BookValueEngine::with_config(BookValueConfig {
            entry_threshold: 0.01, // very low to force entry
            ..Default::default()
        });

        // Manually set up position
        engine.active_position = true;
        engine.entry_price = Some(0.50);
        engine.entry_direction = Some(Direction::Yes);
        engine.entry_ts = Some(1000);
        engine.peak_price = Some(0.50);

        // Price rises 8% — should activate trailing
        let obs = make_obs(2000, 0.54, 290);
        let _ = engine.decide(&obs);
        assert!(engine.trailing_active, "Trailing should activate at +8%");

        // Price rises to 0.65 — new peak
        let _ = engine.decide(&make_obs(3000, 0.65, 280));
        assert_eq!(engine.peak_price, Some(0.65));

        // Price drops 5% from peak — should trigger trail
        let trail_trigger = 0.65 * 0.95;
        let decision = engine.decide(&make_obs(4000, trail_trigger - 0.001, 270));
        assert!(matches!(decision, StrategyDecision::Exit { .. }));
    }

    #[test]
    fn test_wider_stop_loss() {
        let mut engine = BookValueEngine::new();

        engine.active_position = true;
        engine.entry_price = Some(0.50);
        engine.entry_direction = Some(Direction::Yes);
        engine.entry_ts = Some(1000);
        engine.peak_price = Some(0.50);

        // 4% drop — should NOT trigger 6% SL
        let decision = engine.decide(&make_obs(2000, 0.48, 290));
        assert!(matches!(decision, StrategyDecision::Hold));

        // Recovery
        let decision = engine.decide(&make_obs(3000, 0.52, 280));
        assert!(matches!(decision, StrategyDecision::Hold));
    }

    #[test]
    fn test_book_health_filter() {
        let mut engine = BookValueEngine::new();
        // Build history
        for i in 0..30 {
            let _ = engine.decide(&make_obs(i * 1000, 0.50, 300));
        }
        // Book sum too high (arbitrage condition)
        let mut obs = make_obs(31000, 0.50, 300);
        obs.book_sum = 1.08;
        let decision = engine.decide(&obs);
        assert!(
            matches!(decision, StrategyDecision::Hold),
            "Should block when book_sum > 1.03"
        );
    }

    #[test]
    fn test_engine_reset() {
        let mut engine = BookValueEngine::new();
        for i in 0..20 {
            let _ = engine.decide(&make_obs(i * 1000, 0.50, 300));
        }
        engine.reset();
        assert!(engine.prev_mid.is_none());
        assert!(!engine.active_position);
        assert_eq!(engine.obi_tracker.values.len(), 0);
    }
}
