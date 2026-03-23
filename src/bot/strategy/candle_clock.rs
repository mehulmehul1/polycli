//! CandleClock Strategy Engine
//!
//! Research-driven time-based strategy combining:
//! 1. Turn-of-candle effect (Shanaev & Vasenin 2023, t-stat > 9)
//! 2. Power hour bias (22:00-23:00 UTC, ~0.07% avg hourly return)
//! 3. Day-of-week patterns (Monday long bias, Sunday evening through Monday)
//! 4. Asian dead zone avoidance (03:00-04:00 UTC negative drift)
//!
//! Key insight from research: time alone is not enough (edge too small for fees).
//! Strategy uses time as a GATE — only enter when time conditions are favorable,
//! combined with a simple momentum/volatility signal.

use super::types::*;
use serde::{Deserialize, Serialize};

/// Configuration for CandleClock strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleClockConfig {
    /// Take profit percentage
    pub take_profit_pct: f64,
    /// Stop loss percentage
    pub stop_loss_pct: f64,
    /// Trailing activation (profit level to start trailing)
    pub trail_activation_pct: f64,
    /// Trailing offset (distance from peak)
    pub trail_offset_pct: f64,
    /// Post-exit cooldown in seconds
    pub cooldown_seconds: i64,
    /// Minimum spread to enter (avoids illiquid markets)
    pub min_spread: f64,
    /// Maximum spread to enter (avoids broken books)
    pub max_spread: f64,
    /// Minimum time remaining to enter
    pub min_time_remaining: i64,
    /// Minimum price to enter (avoids extreme prices)
    pub min_entry_prob: f64,
    /// Maximum price to enter
    pub max_entry_prob: f64,
    /// Boundary window — seconds around :00/:15/:30/:45 to allow entry
    pub boundary_window_sec: i64,
    /// Enable power hour boost (22:00-23:00 UTC)
    pub enable_power_hour: bool,
    /// Enable dead zone block (03:00-04:00 UTC)
    pub enable_dead_zone_block: bool,
    /// Enable Monday long bias
    pub enable_monday_bias: bool,
    /// Minimum confidence to enter (0.0-1.0)
    pub min_confidence: f64,
}

impl Default for CandleClockConfig {
    fn default() -> Self {
        Self {
            take_profit_pct: 0.08,
            stop_loss_pct: 0.06,
            trail_activation_pct: 0.05,
            trail_offset_pct: 0.02,
            cooldown_seconds: 90,
            min_spread: 0.005,
            max_spread: 0.08,
            min_time_remaining: 60,
            min_entry_prob: 0.15,
            max_entry_prob: 0.85,
            boundary_window_sec: 45,
            enable_power_hour: true,
            enable_dead_zone_block: true,
            enable_monday_bias: true,
            min_confidence: 0.50,
        }
    }
}

/// CandleClock engine
pub struct CandleClockEngine {
    config: CandleClockConfig,
    active_position: bool,
    entry_price: Option<f64>,
    entry_direction: Option<Direction>,
    entry_ts: Option<i64>,
    last_exit_ts: Option<i64>,
    peak_price: Option<f64>,
    trailing_active: bool,
    /// Price history for momentum signal (last 3 observations)
    price_history: Vec<f64>,
}

impl CandleClockEngine {
    pub fn new() -> Self {
        Self::with_config(CandleClockConfig::default())
    }

    pub fn with_config(config: CandleClockConfig) -> Self {
        Self {
            config,
            active_position: false,
            entry_price: None,
            entry_direction: None,
            entry_ts: None,
            last_exit_ts: None,
            peak_price: None,
            trailing_active: false,
            price_history: Vec::with_capacity(10),
        }
    }

    /// Check if current UTC time is at a candle boundary
    fn is_at_boundary(&self, ts: i64) -> bool {
        let seconds_in_hour = ts % 3600;
        let minute_of_hour = (seconds_in_hour / 60) % 60;
        let second_of_minute = seconds_in_hour % 60;

        let boundary_minutes = [0i64, 15, 30, 45];
        let is_boundary_minute = boundary_minutes.contains(&minute_of_hour);

        // Allow entry within ±boundary_window_sec of the boundary
        let in_window = second_of_minute <= self.config.boundary_window_sec
            || second_of_minute >= (60 - self.config.boundary_window_sec);

        is_boundary_minute && in_window
    }

    /// Get UTC hour from epoch seconds
    fn utc_hour(&self, ts: i64) -> i64 {
        // epoch seconds → UTC hour (0-23)
        (ts / 3600) % 24
    }

    /// Get UTC weekday from epoch seconds (0=Sunday, 1=Monday, ... 6=Saturday)
    fn utc_weekday(&self, ts: i64) -> i64 {
        // Unix epoch (Jan 1 1970) was a Thursday (day 4)
        // Days since epoch + 4, mod 7
        let days_since_epoch = ts / 86400;
        (days_since_epoch + 4) % 7
    }

    /// Compute entry confidence based on time factors
    fn compute_confidence(&self, ts: i64) -> f64 {
        let hour = self.utc_hour(ts);
        let weekday = self.utc_weekday(ts);
        let mut confidence: f64 = 0.55; // base confidence

        // Power hour boost (22:00-23:00 UTC)
        if self.config.enable_power_hour && hour >= 22 && hour < 23 {
            confidence *= 1.35;
        }

        // Monday long bias
        if self.config.enable_monday_bias && weekday == 1 {
            confidence *= 1.20;
        }

        // Friday evening block (after 20:00 UTC)
        if weekday == 5 && hour >= 20 {
            confidence *= 0.3; // severe penalty, effectively blocked
        }

        // Major boundary boost (:00 and :30 stronger than :15/:45)
        let seconds_in_hour = ts % 3600;
        let minute_of_hour = (seconds_in_hour / 60) % 60;
        if minute_of_hour == 0 || minute_of_hour == 30 {
            confidence *= 1.10;
        }

        confidence
    }

    /// Check if dead zone (03:00-04:00 UTC)
    fn is_dead_zone(&self, ts: i64) -> bool {
        if !self.config.enable_dead_zone_block {
            return false;
        }
        let hour = self.utc_hour(ts);
        hour >= 3 && hour < 4
    }

    /// Simple momentum signal from price history
    /// Positive = price rising, Negative = price falling
    fn momentum_signal(&self) -> f64 {
        if self.price_history.len() < 3 {
            return 0.0;
        }
        let n = self.price_history.len();
        let recent = self.price_history[n - 1];
        let prev = self.price_history[n - 3];
        recent - prev
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

        let move_in_favor = match dir {
            Direction::Yes => p - entry,
            Direction::No => entry - p,
        };
        let pnl_pct = move_in_favor * 100.0;

        if obs.time_remaining_s < 15 {
            return Some(ExitReason::TimeExpiry {
                seconds_remaining: obs.time_remaining_s,
            });
        }

        // Track peak
        let is_new_peak = match dir {
            Direction::Yes => self.peak_price.map_or(true, |peak| p > peak),
            Direction::No => self.peak_price.map_or(true, |peak| p < peak),
        };
        if is_new_peak {
            self.peak_price = Some(p);
        }

        // Activate trailing
        if !self.trailing_active && move_in_favor >= self.config.trail_activation_pct {
            self.trailing_active = true;
        }

        // Trailing stop
        if self.trailing_active {
            let peak = self.peak_price.unwrap_or(entry);
            let trail_trigger = match dir {
                Direction::Yes => peak * (1.0 - self.config.trail_offset_pct),
                Direction::No => peak * (1.0 + self.config.trail_offset_pct),
            };
            let hit = match dir {
                Direction::Yes => p <= trail_trigger,
                Direction::No => p >= trail_trigger,
            };
            if hit {
                return Some(ExitReason::TakeProfit { pnl_pct });
            }
        }

        // Fixed TP
        if !self.trailing_active && move_in_favor >= self.config.take_profit_pct {
            return Some(ExitReason::TakeProfit { pnl_pct });
        }

        // SL
        if move_in_favor <= -self.config.stop_loss_pct {
            return Some(ExitReason::StopLoss { pnl_pct });
        }

        // Extreme price
        if p > 0.95 || p < 0.05 {
            return Some(ExitReason::RiskGate {
                reason: format!("extreme {:.3}", p),
            });
        }

        None
    }

    fn reset_position(&mut self, ts: i64) {
        self.active_position = false;
        self.entry_price = None;
        self.entry_direction = None;
        self.entry_ts = None;
        self.last_exit_ts = Some(ts);
        self.peak_price = None;
        self.trailing_active = false;
    }
}

impl super::StrategyEngine for CandleClockEngine {
    fn decide(&mut self, obs: &super::Observation) -> StrategyDecision {
        let p = obs.yes_mid;

        // Track price history
        self.price_history.push(p);
        if self.price_history.len() > 10 {
            self.price_history.remove(0);
        }

        // Check exit if position active
        if self.active_position {
            if let Some(exit_reason) = self.check_exit(obs) {
                self.reset_position(obs.ts);
                return StrategyDecision::Exit {
                    position_id: String::new(),
                    reason: exit_reason,
                };
            }
            return StrategyDecision::Hold;
        }

        // Hard blocks
        if self.is_dead_zone(obs.ts) {
            return StrategyDecision::Hold;
        }

        // Cooldown
        if let Some(last_exit) = self.last_exit_ts {
            if obs.ts - last_exit < self.config.cooldown_seconds {
                return StrategyDecision::Hold;
            }
        }

        // Must be at candle boundary
        if !self.is_at_boundary(obs.ts) {
            return StrategyDecision::Hold;
        }

        // Price range filter
        if p > self.config.max_entry_prob || p < self.config.min_entry_prob {
            return StrategyDecision::Hold;
        }

        // Spread filter
        let spread = obs.yes_ask - obs.yes_bid;
        if spread < self.config.min_spread || spread > self.config.max_spread {
            return StrategyDecision::Hold;
        }

        // Time remaining
        if obs.time_remaining_s < self.config.min_time_remaining {
            return StrategyDecision::Hold;
        }

        // Need price history
        if self.price_history.len() < 3 {
            return StrategyDecision::Hold;
        }

        // Compute confidence
        let confidence = self.compute_confidence(obs.ts);

        // Momentum signal — only enter in direction of short-term momentum
        let momentum = self.momentum_signal();

        // Entry decision: time gate passes + confidence threshold + momentum confirms
        if confidence >= self.config.min_confidence {
            let direction = if momentum >= 0.0 {
                Direction::Yes
            } else {
                // Momentum is negative — skip entry, wait for reversal
                return StrategyDecision::Hold;
            };

            self.active_position = true;
            self.entry_price = Some(obs.yes_ask);
            self.entry_direction = Some(direction);
            self.entry_ts = Some(obs.ts);
            self.peak_price = Some(p);
            self.trailing_active = false;

            let hour = self.utc_hour(obs.ts);
            let weekday = self.utc_weekday(obs.ts);

            return StrategyDecision::Enter {
                direction,
                reason: EntryReason {
                    source: SignalSource::Fused,
                    confidence: Confidence::new(confidence.min(1.0)),
                    detail: format!(
                        "CandleClock: h{} d{} conf={:.2} mom={:.4}",
                        hour, weekday, confidence, momentum
                    ),
                    fair_value_edge: None,
                    qlib_score: None,
                },
            };
        }

        StrategyDecision::Hold
    }

    fn reset(&mut self) {
        self.active_position = false;
        self.entry_price = None;
        self.entry_direction = None;
        self.entry_ts = None;
        self.last_exit_ts = None;
        self.peak_price = None;
        self.trailing_active = false;
        self.price_history.clear();
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
    fn test_boundary_detection() {
        let engine = CandleClockEngine::new();
        // :00 boundary (0 seconds into hour)
        assert!(engine.is_at_boundary(0));
        // :15 boundary
        assert!(engine.is_at_boundary(15 * 60));
        // :30 boundary
        assert!(engine.is_at_boundary(30 * 60));
        // :45 boundary
        assert!(engine.is_at_boundary(45 * 60));
        // :07 (not a boundary)
        assert!(!engine.is_at_boundary(7 * 60 + 30));
    }

    #[test]
    fn test_dead_zone_block() {
        let engine = CandleClockEngine::new();
        // 03:30 UTC = 3*3600 + 30*60 = 12600
        assert!(engine.is_dead_zone(12600));
        // 04:30 UTC = 4*3600 + 30*60 = 16200
        assert!(!engine.is_dead_zone(16200));
        // 22:30 UTC = 22*3600 + 30*60 = 81000
        assert!(!engine.is_dead_zone(81000));
    }

    #[test]
    fn test_power_hour_confidence() {
        let engine = CandleClockEngine::new();
        // 22:00 UTC = power hour
        let conf_power = engine.compute_confidence(22 * 3600);
        // 10:00 UTC = normal hour
        let conf_normal = engine.compute_confidence(10 * 3600);
        assert!(
            conf_power > conf_normal,
            "Power hour should have higher confidence"
        );
    }

    #[test]
    fn test_no_entry_extreme_price() {
        let mut engine = CandleClockEngine::new();
        // Build price history at boundary
        for i in 0..5 {
            let ts = i * 60; // every minute
            let _ = engine.decide(&make_obs(ts, 0.50, 300));
        }
        // Try entry at 0.92 — should be blocked
        let obs = make_obs(300, 0.92, 300);
        let decision = engine.decide(&obs);
        assert!(matches!(decision, StrategyDecision::Hold));
    }

    #[test]
    fn test_trailing_stop() {
        let mut engine = CandleClockEngine::new();
        engine.active_position = true;
        engine.entry_price = Some(0.50);
        engine.entry_direction = Some(Direction::Yes);
        engine.entry_ts = Some(1000);
        engine.peak_price = Some(0.50);

        // +8% — should activate trailing
        let _ = engine.decide(&make_obs(2000, 0.54, 290));
        assert!(engine.trailing_active);

        // Peak to 0.65
        let _ = engine.decide(&make_obs(3000, 0.65, 280));
        assert_eq!(engine.peak_price, Some(0.65));

        // Drop 2% from peak → trail trigger
        let decision = engine.decide(&make_obs(4000, 0.65 * 0.98 - 0.001, 270));
        assert!(matches!(decision, StrategyDecision::Exit { .. }));
    }

    #[test]
    fn test_engine_reset() {
        let mut engine = CandleClockEngine::new();
        for i in 0..10 {
            let _ = engine.decide(&make_obs(i * 60, 0.50, 300));
        }
        engine.reset();
        assert!(!engine.active_position);
        assert!(engine.price_history.is_empty());
    }
}
