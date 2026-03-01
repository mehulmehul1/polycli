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
        }
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
        // Trade only mid probability range (avoid edge instability)
        if p < 0.35 || p > 0.65 {
            return EntrySignal::None;
        }

        // Require minimum window size
        if self.recent_5s_closes.len() < 5 {
            return EntrySignal::None;
        }

        let slope_5s = match five_sec.momentum_slope {
            Some(s) => s,
            None => return EntrySignal::None,
        };

        // Optional 1m regime filter (slope sign only)
        if let Some(slope_1m) = one_min.momentum_slope {
            if slope_1m.abs() < 0.0005 {
                return EntrySignal::None;
            }
        }

        // Upward probability expansion
        if slope_5s > 0.002 && p > self.last_high {
            return EntrySignal::Long;
        }

        // Downward probability expansion
        if slope_5s < -0.002 && p < self.last_low {
            return EntrySignal::Short;
        }

        EntrySignal::None
    }

    fn check_exit(&self, five_sec: &IndicatorState) -> ExitSignal {
        let slope = match five_sec.momentum_slope {
            Some(s) => s,
            None => return ExitSignal::None,
        };

        match self.active_position {
            Some(EntrySignal::Long) => {
                if slope < 0.0 {
                    return ExitSignal::FullExit;
                }
            }
            Some(EntrySignal::Short) => {
                if slope > 0.0 {
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

    fn mock_state(slope: Option<f64>) -> IndicatorState {
        IndicatorState {
            ema9: None,
            ema21: None,
            rsi14: None,
            momentum_slope: slope,
        }
    }

    #[test]
    fn long_expansion_entry() {
        let mut engine = SignalEngine::new();

        let five_sec = mock_state(Some(0.005));
        let one_min = mock_state(Some(0.001));

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

        let five_sec = mock_state(Some(-0.005));
        let one_min = mock_state(Some(-0.001));

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

        let five_sec = mock_state(Some(-0.001));
        let one_min = mock_state(Some(0.0));

        let state = engine.update(&five_sec, &one_min, 0.55);
        assert_eq!(state.exit, ExitSignal::FullExit);
    }

    #[test]
    fn no_entry_outside_probability_range() {
        let mut engine = SignalEngine::new();

        let five_sec = mock_state(Some(0.01));
        let one_min = mock_state(Some(0.001));

        let state = engine.update(&five_sec, &one_min, 0.30);
        assert_eq!(state.entry, EntrySignal::None);

        let state = engine.update(&five_sec, &one_min, 0.70);
        assert_eq!(state.entry, EntrySignal::None);
    }
}
