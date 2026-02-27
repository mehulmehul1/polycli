use crate::bot::indicators::IndicatorState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    Long,
    Short,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySignal {
    Long,
    Short,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitSignal {
    ScaleOut25,
    ScaleOut50,
    FullExit,
    StopLoss,
    None,
}

#[derive(Debug, Clone)]
pub struct SignalState {
    pub bias: Bias,
    pub acceleration: bool,
    pub entry: EntrySignal,
    pub exit: ExitSignal,
}

pub struct SignalEngine {
    previous_bias: Bias,
    active_position: Option<Bias>,
    entry_price: Option<f64>,
    scale_stage: u8,
    
    // Internal state tracking for 5s breakout
    last_5s_high: f64,
    last_5s_low: f64,
    recent_5s_closes: std::collections::VecDeque<f64>,
}

impl SignalEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            previous_bias: Bias::Neutral,
            active_position: None,
            entry_price: None,
            scale_stage: 0,
            last_5s_high: f64::MIN,
            last_5s_low: f64::MAX,
            recent_5s_closes: std::collections::VecDeque::with_capacity(3),
        }
    }

    pub fn update(
        &mut self,
        one_min: &IndicatorState,
        fifteen_sec: &IndicatorState,
        five_sec: &IndicatorState,
        current_price: f64,
    ) -> SignalState {
        // 1. Structural Bias (1m)
        let bias = Self::determine_bias(one_min);
        self.previous_bias = bias;

        // 2. Acceleration (15s)
        let acceleration = Self::check_acceleration(bias, fifteen_sec);

        // 4. Micro Entry (5s) - Calculate breakout using history BEFORE pushing current price
        self.last_5s_high = self.recent_5s_closes.iter().copied().fold(f64::MIN, f64::max);
        self.last_5s_low = self.recent_5s_closes.iter().copied().fold(f64::MAX, f64::min);
        
        let entry = self.check_entry(bias, acceleration, five_sec, current_price);

        // Update 5s history queue for NEXT tick
        if self.recent_5s_closes.len() == 3 {
            self.recent_5s_closes.pop_front();
        }
        self.recent_5s_closes.push_back(current_price);

        // 3. Position Management / Exits
        let exit = self.check_exit(current_price, one_min);

        // Track internal position state based on active exit signals
        match exit {
            ExitSignal::FullExit | ExitSignal::StopLoss => {
                self.active_position = None;
                self.entry_price = None;
                self.scale_stage = 0;
            }
            ExitSignal::ScaleOut25 => self.scale_stage = 1,
            ExitSignal::ScaleOut50 => self.scale_stage = 2,
            ExitSignal::None => {}
        }

        // Track internal position state based on active entry signals
        if entry != EntrySignal::None && self.active_position.is_none() {
            self.active_position = match entry {
                EntrySignal::Long => Some(Bias::Long),
                EntrySignal::Short => Some(Bias::Short),
                EntrySignal::None => None,
            };
            self.entry_price = Some(current_price);
            self.scale_stage = 0;
        }

        SignalState {
            bias,
            acceleration,
            entry,
            exit,
        }
    }

    fn determine_bias(one_min: &IndicatorState) -> Bias {
        if let (Some(ema9), Some(ema21), Some(rsi), Some(slope)) = 
            (one_min.ema9, one_min.ema21, one_min.rsi14, one_min.momentum_slope) {
            
            if ema9 > ema21 && rsi > 55.0 && slope > 0.0 {
                return Bias::Long;
            }
            if ema9 < ema21 && rsi < 45.0 && slope < 0.0 {
                return Bias::Short;
            }
        }
        Bias::Neutral
    }

    fn check_acceleration(bias: Bias, fifteen_sec: &IndicatorState) -> bool {
        if bias == Bias::Neutral {
            return false;
        }

        if let (Some(ema9), Some(ema21), Some(rsi), Some(slope)) = 
            (fifteen_sec.ema9, fifteen_sec.ema21, fifteen_sec.rsi14, fifteen_sec.momentum_slope) {
            
            return match bias {
                Bias::Long => slope > 0.0 && rsi > 60.0 && ema9 > ema21,
                Bias::Short => slope < 0.0 && rsi < 40.0 && ema9 < ema21,
                Bias::Neutral => false,
            };
        }
        false
    }

    fn check_entry(
        &self, 
        bias: Bias, 
        acceleration: bool, 
        five_sec: &IndicatorState, 
        current_price: f64
    ) -> EntrySignal {
        if self.active_position.is_some() || !acceleration || bias == Bias::Neutral || self.recent_5s_closes.len() < 3 {
            return EntrySignal::None;
        }

        if let Some(slope) = five_sec.momentum_slope {
            match bias {
                Bias::Long => {
                    if slope > 0.001 && current_price > self.last_5s_high {
                        return EntrySignal::Long;
                    }
                }
                Bias::Short => {
                    if slope < -0.001 && current_price < self.last_5s_low {
                        return EntrySignal::Short;
                    }
                }
                Bias::Neutral => {}
            }
        }
        EntrySignal::None
    }

    fn check_exit(&self, current_price: f64, one_min: &IndicatorState) -> ExitSignal {
        if let (Some(position), Some(entry)) = (self.active_position, self.entry_price) {
            let pct_change = (current_price - entry) / entry;

            // 1m slope flip logic for immediate full exit
            if let Some(slope) = one_min.momentum_slope {
                if position == Bias::Long && slope < 0.0 {
                    return ExitSignal::FullExit;
                }
                if position == Bias::Short && slope > 0.0 {
                    return ExitSignal::FullExit;
                }
            }

            match position {
                Bias::Long => {
                    if pct_change <= -0.10 + 1e-9 { return ExitSignal::StopLoss; }
                    if current_price >= 0.94 { return ExitSignal::FullExit; }
                    if pct_change >= 0.40 - 1e-9 && self.scale_stage < 2 { return ExitSignal::ScaleOut50; }
                    if pct_change >= 0.20 - 1e-9 && self.scale_stage < 1 { return ExitSignal::ScaleOut25; }
                }
                Bias::Short => {
                    if pct_change >= 0.10 - 1e-9 { return ExitSignal::StopLoss; }
                    // Synthesizing a short floor (e.g. 0.06 as equivalent to 0.94 ceiling for longs)
                    if current_price <= 0.06 { return ExitSignal::FullExit; } 
                    if pct_change <= -0.40 + 1e-9 && self.scale_stage < 2 { return ExitSignal::ScaleOut50; }
                    if pct_change <= -0.20 + 1e-9 && self.scale_stage < 1 { return ExitSignal::ScaleOut25; }
                }
                Bias::Neutral => {}
            }
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

    fn mock_state(ema9: Option<f64>, ema21: Option<f64>, rsi14: Option<f64>, slope: Option<f64>) -> IndicatorState {
        IndicatorState {
            ema9,
            ema21,
            rsi14,
            momentum_slope: slope,
        }
    }

    #[test]
    fn long_trend_entry() {
        let mut engine = SignalEngine::new();
        
        let one_m = mock_state(Some(0.60), Some(0.55), Some(58.0), Some(0.01));
        let fifteen_s = mock_state(Some(0.62), Some(0.60), Some(65.0), Some(0.02));
        let five_s = mock_state(Some(0.63), Some(0.61), Some(68.0), Some(0.005));

        // Push 3 candles to build 5s history (requires 3 for breakout check)
        engine.update(&one_m, &fifteen_s, &five_s, 0.59);
        engine.update(&one_m, &fifteen_s, &five_s, 0.60);
        engine.update(&one_m, &fifteen_s, &five_s, 0.61);
        
        // Push 4th candle to break high and trigger entry
        let state = engine.update(&one_m, &fifteen_s, &five_s, 0.65);

        assert_eq!(state.bias, Bias::Long);
        assert!(state.acceleration);
        assert_eq!(state.entry, EntrySignal::Long);
        assert_eq!(state.exit, ExitSignal::None);
        assert_eq!(engine.active_position, Some(Bias::Long));
    }

    #[test]
    fn no_entry_in_neutral() {
        let mut engine = SignalEngine::new();
        
        let one_m = mock_state(Some(0.50), Some(0.50), Some(50.0), Some(0.0));
        let fifteen_s = mock_state(Some(0.51), Some(0.50), Some(55.0), Some(0.001));
        let five_s = mock_state(Some(0.55), Some(0.50), Some(70.0), Some(0.01));

        engine.update(&one_m, &fifteen_s, &five_s, 0.50);
        engine.update(&one_m, &fifteen_s, &five_s, 0.51);
        let state = engine.update(&one_m, &fifteen_s, &five_s, 0.55);

        assert_eq!(state.bias, Bias::Neutral);
        assert!(!state.acceleration);
        assert_eq!(state.entry, EntrySignal::None);
        assert_eq!(engine.active_position, None);
    }

    #[test]
    fn scale_out_sequence() {
        let mut engine = SignalEngine::new();
        engine.active_position = Some(Bias::Long);
        engine.entry_price = Some(0.50); // Base entry

        let neutral_1m = mock_state(Some(0.60), Some(0.55), Some(60.0), Some(0.01));
        
        // 1. +20% -> 0.60
        let state1 = engine.update(&neutral_1m, &neutral_1m, &neutral_1m, 0.60);
        assert_eq!(state1.exit, ExitSignal::ScaleOut25);
        assert_eq!(engine.scale_stage, 1);

        // 2. Stay at 0.60 -> Should be None (already scaled)
        let state2 = engine.update(&neutral_1m, &neutral_1m, &neutral_1m, 0.60);
        assert_eq!(state2.exit, ExitSignal::None);

        // 3. +40% -> 0.70
        let state3 = engine.update(&neutral_1m, &neutral_1m, &neutral_1m, 0.70);
        assert_eq!(state3.exit, ExitSignal::ScaleOut50);
        assert_eq!(engine.scale_stage, 2);

        // 4. Hit 0.94 ceiling -> FullExit
        let state4 = engine.update(&neutral_1m, &neutral_1m, &neutral_1m, 0.95);
        assert_eq!(state4.exit, ExitSignal::FullExit);
        assert_eq!(engine.active_position, None);
    }

    #[test]
    fn stop_loss_trigger() {
        let mut engine = SignalEngine::new();
        engine.active_position = Some(Bias::Long);
        engine.entry_price = Some(0.50);

        let neutral_1m = mock_state(Some(0.50), Some(0.50), Some(50.0), Some(0.0));
        
        // -10% -> 0.45
        let state = engine.update(&neutral_1m, &neutral_1m, &neutral_1m, 0.45);
        
        assert_eq!(state.exit, ExitSignal::StopLoss);
        assert_eq!(engine.active_position, None);
    }

    #[test]
    fn slope_flip_exit() {
        let mut engine = SignalEngine::new();
        engine.active_position = Some(Bias::Long);
        engine.entry_price = Some(0.50);

        // Still in profit (0.55), but 1m slope flipped completely negative
        let bearish_1m = mock_state(Some(0.50), Some(0.55), Some(40.0), Some(-0.01));
        
        let state = engine.update(&bearish_1m, &bearish_1m, &bearish_1m, 0.55);
        
        assert_eq!(state.exit, ExitSignal::FullExit);
        assert_eq!(engine.active_position, None);
    }
}
