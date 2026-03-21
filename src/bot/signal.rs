use crate::bot::indicators::IndicatorState;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySignal {
    Long,  // Buy YES
    Short, // Buy NO
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitSignal {
    FullExit,
    None,
}

#[derive(Debug, Clone)]
pub struct SignalState {
    pub entry: EntrySignal,
    pub exit: ExitSignal,
}

pub struct SignalEngine {
    active_position: Option<EntrySignal>,
    entry_price: Option<f64>,

    recent_5s_closes: VecDeque<f64>,
    last_high: f64,
    last_low: f64,

    entry_band_low: f64,
    entry_band_high: f64,
}

impl SignalEngine {
    #[must_use]
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

    #[must_use]
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

    pub fn reset(&mut self) {
        self.active_position = None;
        self.entry_price = None;
        self.recent_5s_closes.clear();
        self.last_high = f64::MIN;
        self.last_low = f64::MAX;
    }

    pub fn update(
        &mut self,
        five_sec: &IndicatorState,
        one_min: &IndicatorState,
        midpoint_probability: f64,
    ) -> SignalState {
        // Calculate high/low from PREVIOUS window (before adding current)
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

        // Now add current price to window
        if self.recent_5s_closes.len() == 5 {
            self.recent_5s_closes.pop_front();
        }
        self.recent_5s_closes.push_back(midpoint_probability);

        let entry = self.check_entry(five_sec, one_min, midpoint_probability);
        let exit = self.check_exit(five_sec);

        // Track internal position state
        match exit {
            ExitSignal::FullExit => {
                self.active_position = None;
                self.entry_price = None;
            }
            ExitSignal::None => {}
        }

        if entry != EntrySignal::None && self.active_position.is_none() {
            self.active_position = Some(entry);
            self.entry_price = Some(midpoint_probability);
        }

        SignalState { entry, exit }
    }

    fn check_entry(
        &self,
        five_sec: &IndicatorState,
        one_min: &IndicatorState,
        p: f64,
    ) -> EntrySignal {
        if p > 0.92 || p < 0.08 {
            return EntrySignal::None;
        }

        // Entry band filter: only enter when midpoint is within configured band
        if p < self.entry_band_low || p > self.entry_band_high {
            return EntrySignal::None;
        }

        // Require minimum window size
        if self.recent_5s_closes.len() < 5 {
            return EntrySignal::None;
        }

        let rsi = five_sec.rsi14.unwrap_or(50.0);
        let slope = five_sec.momentum_slope.unwrap_or(0.0);

        let ema_fast = five_sec.ema3.unwrap_or(p);
        let ema_slow = five_sec.ema6.unwrap_or(p);

        let bb_width = five_sec.bb_width.unwrap_or(0.0);
        let bbp = five_sec.bb_percent.unwrap_or(0.5);

        if bb_width < 0.15 {
            // Log near misses for BB width if it's close
            if bb_width > 0.05 {
                println!(
                    "[DEBUG] Signal blocked by BB_WIDTH: {:.4} < 0.15 (Slope: {:.4}, RSI: {:.2})",
                    bb_width, slope, rsi
                );
            }
            return EntrySignal::None;
        }

        // Expansion trade
        if bb_width >= 0.15 && bb_width < 0.9 {
            if ema_fast > ema_slow && slope > 0.002 && rsi < 70.0 {
                return EntrySignal::Long;
            }

            if ema_fast < ema_slow && slope < -0.002 && rsi > 30.0 {
                return EntrySignal::Short;
            }
        }

        // Reversal trade
        if bb_width >= 0.9 {
            if rsi > 75.0 && bbp > 0.9 && ema_fast < ema_slow {
                return EntrySignal::Short;
            }

            if rsi < 25.0 && bbp < 0.1 && ema_fast > ema_slow {
                return EntrySignal::Long;
            }
        }

        EntrySignal::None
    }

    fn check_exit(&self, five_sec: &IndicatorState) -> ExitSignal {
        let ema_fast = five_sec.ema3.unwrap_or(0.0);
        let ema_slow = five_sec.ema6.unwrap_or(0.0);
        let slope = five_sec.momentum_slope.unwrap_or(0.0);

        match self.active_position {
            Some(EntrySignal::Long) => {
                if ema_fast < ema_slow || slope < 0.0 {
                    return ExitSignal::FullExit;
                }
            }
            Some(EntrySignal::Short) => {
                if ema_fast > ema_slow || slope > 0.0 {
                    return ExitSignal::FullExit;
                }
            }
            _ => {}
        }

        ExitSignal::None
    }
}

impl Default for SignalEngine {
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
            bb_width: Some(0.5), // so it doesn't fail minimum vol test by default
            bb_percent: Some(0.5),
            williams_r: None,
        }
    }

    #[test]
    fn long_expansion_entry() {
        let mut engine = SignalEngine::new();

        let five_sec = mock_state(Some(0.005), Some(0.51), Some(0.50));
        let one_min = mock_state(Some(0.001), Some(0.51), Some(0.50));

        let state = engine.update(&five_sec, &one_min, 0.50);
        assert_eq!(state.entry, EntrySignal::None);

        for p in [0.50, 0.51, 0.52, 0.53, 0.54] {
            let state = engine.update(&five_sec, &one_min, p);
        }

        let state = engine.update(&five_sec, &one_min, 0.55);
        assert_eq!(state.entry, EntrySignal::Long);
    }

    #[test]
    fn short_expansion_entry() {
        let mut engine = SignalEngine::new();

        let five_sec = mock_state(Some(-0.005), Some(0.49), Some(0.50));
        let one_min = mock_state(Some(-0.001), Some(0.49), Some(0.50));

        for p in [0.50, 0.49, 0.48, 0.47, 0.46] {
            let state = engine.update(&five_sec, &one_min, p);
        }

        let state = engine.update(&five_sec, &one_min, 0.45);
        assert_eq!(state.entry, EntrySignal::Short);
    }

    #[test]
    fn slope_flip_exit_long() {
        let mut engine = SignalEngine::new();
        engine.active_position = Some(EntrySignal::Long);

        let five_sec = mock_state(Some(-0.001), Some(0.49), Some(0.50));
        let one_min = mock_state(Some(0.0), Some(0.50), Some(0.50));

        let state = engine.update(&five_sec, &one_min, 0.55);
        assert_eq!(state.exit, ExitSignal::FullExit);
    }

    #[test]
    fn no_entry_outside_probability_range() {
        let mut engine = SignalEngine::new();
        let five_sec = mock_state(Some(0.005), Some(0.51), Some(0.50));
        let one_min = mock_state(Some(0.001), Some(0.51), Some(0.50));

        // Baseline established at 0.50
        engine.update(&five_sec, &one_min, 0.50);

        // Probability moves to 0.95 (outside loop safety filter 0.08 - 0.92)
        let state = engine.update(&five_sec, &one_min, 0.95);
        assert_eq!(state.entry, EntrySignal::None);
    }
}
