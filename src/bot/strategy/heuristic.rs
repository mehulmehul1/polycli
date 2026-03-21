//! Heuristic Strategy Engine
//!
//! Indicator-based entry/exit logic migrated from signal.rs

use super::{
    Confidence, Direction, EntryReason, EntrySignal, ExitReason, ExitSignal, Observation,
    SignalSource, StrategyDecision, StrategyEngine,
};
use crate::bot::indicators::IndicatorState;
use std::collections::VecDeque;

/// Heuristic engine using technical indicators
pub struct HeuristicEngine {
    active_position: Option<EntrySignal>,
    entry_price: Option<f64>,
    recent_5s_closes: VecDeque<f64>,
    last_high: f64,
    last_low: f64,
    entry_band_low: f64,
    entry_band_high: f64,
}

impl HeuristicEngine {
    pub fn new() -> Self {
        Self {
            active_position: None,
            entry_price: None,
            recent_5s_closes: VecDeque::with_capacity(5),
            last_high: f64::MIN,
            last_low: f64::MAX,
            entry_band_low: 0.35,
            entry_band_high: 0.65,
        }
    }

    pub fn new_with_band(low: f64, high: f64) -> Self {
        Self {
            active_position: None,
            entry_price: None,
            recent_5s_closes: VecDeque::with_capacity(5),
            last_high: f64::MIN,
            last_low: f64::MAX,
            entry_band_low: low,
            entry_band_high: high,
        }
    }

    pub fn set_entry_band(&mut self, low: f64, high: f64) {
        self.entry_band_low = low;
        self.entry_band_high = high;
    }

    fn check_entry(
        &self,
        five_sec: &IndicatorState,
        one_min: &IndicatorState,
        p: f64,
    ) -> Option<(Direction, EntryReason)> {
        // Price range filter
        if p > 0.92 || p < 0.08 {
            return None;
        }

        // Require minimum window size
        if self.recent_5s_closes.len() < 5 {
            return None;
        }

        let rsi = five_sec.rsi14.unwrap_or(50.0);
        let slope = five_sec.momentum_slope.unwrap_or(0.0);
        let ema_fast = five_sec.ema3.unwrap_or(p);
        let ema_slow = five_sec.ema6.unwrap_or(p);
        let bb_width = five_sec.bb_width.unwrap_or(0.0);
        let bbp = five_sec.bb_percent.unwrap_or(0.5);

        // BB width filter
        if bb_width < 0.15 {
            return None;
        }

        // Expansion trade
        if bb_width >= 0.15 && bb_width < 0.9 {
            if ema_fast > ema_slow && slope > 0.002 && rsi < 70.0 {
                return Some((
                    Direction::Yes,
                    EntryReason {
                        source: SignalSource::Indicators,
                        confidence: Confidence::new(0.6 + slope * 10.0),
                        detail: format!(
                            "Long expansion: EMA cross, slope={:.4}, RSI={:.1}",
                            slope, rsi
                        ),
                        fair_value_edge: None,
                        qlib_score: None,
                    },
                ));
            }

            if ema_fast < ema_slow && slope < -0.002 && rsi > 30.0 {
                return Some((
                    Direction::No,
                    EntryReason {
                        source: SignalSource::Indicators,
                        confidence: Confidence::new(0.6 + slope.abs() * 10.0),
                        detail: format!(
                            "Short expansion: EMA cross, slope={:.4}, RSI={:.1}",
                            slope, rsi
                        ),
                        fair_value_edge: None,
                        qlib_score: None,
                    },
                ));
            }
        }

        // Reversal trade
        if bb_width >= 0.9 {
            if rsi > 75.0 && bbp > 0.9 && ema_fast < ema_slow {
                return Some((
                    Direction::No,
                    EntryReason {
                        source: SignalSource::Indicators,
                        confidence: Confidence::HIGH,
                        detail: format!("Reversal short: RSI={:.1}, BBP={:.2}", rsi, bbp),
                        fair_value_edge: None,
                        qlib_score: None,
                    },
                ));
            }

            if rsi < 25.0 && bbp < 0.1 && ema_fast > ema_slow {
                return Some((
                    Direction::Yes,
                    EntryReason {
                        source: SignalSource::Indicators,
                        confidence: Confidence::HIGH,
                        detail: format!("Reversal long: RSI={:.1}, BBP={:.2}", rsi, bbp),
                        fair_value_edge: None,
                        qlib_score: None,
                    },
                ));
            }
        }

        None
    }

    fn check_exit(&self, five_sec: &IndicatorState) -> Option<ExitReason> {
        let ema_fast = five_sec.ema3.unwrap_or(0.0);
        let ema_slow = five_sec.ema6.unwrap_or(0.0);
        let slope = five_sec.momentum_slope.unwrap_or(0.0);

        match self.active_position {
            Some(EntrySignal::Long) => {
                if ema_fast < ema_slow || slope < 0.0 {
                    return Some(ExitReason::MomentumReversal);
                }
            }
            Some(EntrySignal::Short) => {
                if ema_fast > ema_slow || slope > 0.0 {
                    return Some(ExitReason::MomentumReversal);
                }
            }
            _ => {}
        }

        None
    }
}

impl StrategyEngine for HeuristicEngine {
    fn decide(&mut self, obs: &super::Observation) -> StrategyDecision {
        let p = obs.yes_mid;

        // Update high/low from previous window
        self.last_high = self
            .recent_5s_closes
            .iter()
            .copied()
            .fold(f64::MIN, f64::max);
        self.last_low = self
            .recent_5s_closes
            .iter()
            .copied()
            .fold(f64::MAX, f64::min);

        // Add current price to window
        if self.recent_5s_closes.len() == 5 {
            self.recent_5s_closes.pop_front();
        }
        self.recent_5s_closes.push_back(p);

        // Check for exit first
        if let Some(reason) = self.check_exit(&obs.indicator_5s) {
            self.active_position = None;
            self.entry_price = None;
            return StrategyDecision::Exit {
                position_id: obs.condition_id.clone(),
                reason,
            };
        }

        // Check for entry
        if self.active_position.is_none() {
            if let Some((direction, reason)) =
                self.check_entry(&obs.indicator_5s, &obs.indicator_1m, p)
            {
                self.active_position = Some(match direction {
                    Direction::Yes => EntrySignal::Long,
                    Direction::No => EntrySignal::Short,
                });
                self.entry_price = Some(p);
                return StrategyDecision::Enter { direction, reason };
            }
        }

        StrategyDecision::Hold
    }

    fn reset(&mut self) {
        self.active_position = None;
        self.entry_price = None;
        self.recent_5s_closes.clear();
        self.last_high = f64::MIN;
        self.last_low = f64::MAX;
    }
}

impl Default for HeuristicEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_state(
        slope: Option<f64>,
        ema_fast: Option<f64>,
        ema_slow: Option<f64>,
    ) -> IndicatorState {
        IndicatorState {
            ema3: ema_fast,
            ema6: ema_slow,
            ema9: None,
            ema21: None,
            rsi14: Some(50.0),
            momentum_slope: slope,
            bb_width: Some(0.5),
            bb_percent: Some(0.5),
            williams_r: None,
        }
    }

    fn mock_obs(p: f64, state: &IndicatorState) -> Observation {
        Observation {
            ts: 0,
            condition_id: "test".to_string(),
            market_slug: "test-market".to_string(),
            yes_bid: p - 0.01,
            yes_ask: p + 0.01,
            no_bid: 1.0 - p - 0.01,
            no_ask: 1.0 - p + 0.01,
            yes_mid: p,
            no_mid: 1.0 - p,
            book_sum: 1.0,
            time_remaining_s: 300,
            indicator_5s: state.clone(),
            indicator_1m: state.clone(),
            fair_value_prob: None,
            qlib_score: None,
        }
    }

    #[test]
    fn no_entry_without_window() {
        let mut engine = HeuristicEngine::new();
        let state = mock_state(Some(0.005), Some(0.51), Some(0.50));
        let obs = mock_obs(0.50, &state);
        let decision = engine.decide(&obs);
        assert!(matches!(decision, StrategyDecision::Hold));
    }

    #[test]
    fn long_entry_after_window() {
        let mut engine = HeuristicEngine::new();
        let state = mock_state(Some(0.005), Some(0.51), Some(0.50));

        // Build up window
        for p in [0.50, 0.51, 0.52, 0.53, 0.54] {
            let obs = mock_obs(p, &state);
            engine.decide(&obs);
        }

        // Next tick should trigger entry
        let obs = mock_obs(0.55, &state);
        let decision = engine.decide(&obs);
        assert!(matches!(decision, StrategyDecision::Enter { .. }));
    }
}
