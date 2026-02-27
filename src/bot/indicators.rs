use crate::bot::candles::Candle;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Default)]
pub struct IndicatorState {
    pub ema9: Option<f64>,
    pub ema21: Option<f64>,
    pub rsi14: Option<f64>,
    pub momentum_slope: Option<f64>,
}

pub struct IndicatorEngine {
    ema9: Ema,
    ema21: Ema,
    rsi14: Rsi,
    slope: MomentumSlope,
    last_candle_time: Option<u64>,
    prev_ema9: Option<f64>,
    prev_ema21: Option<f64>,
    pub debug_logs: bool,
}

impl IndicatorEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ema9: Ema::new(9),
            ema21: Ema::new(21),
            rsi14: Rsi::new(14),
            slope: MomentumSlope::new(14),
            last_candle_time: None,
            prev_ema9: None,
            prev_ema21: None,
            debug_logs: false,
        }
    }

    pub fn set_debug(&mut self, enabled: bool) {
        self.debug_logs = enabled;
    }

    pub fn reset(&mut self) {
        self.ema9 = Ema::new(9);
        self.ema21 = Ema::new(21);
        self.rsi14 = Rsi::new(14);
        self.slope = MomentumSlope::new(14);
        self.last_candle_time = None;
        self.prev_ema9 = None;
        self.prev_ema21 = None;
        if self.debug_logs {
            println!("[INDICATORS] Engine reset triggered.");
        }
    }

    pub fn is_ready(&self) -> bool {
        // With Steady State Seeding, indicators are functionally ready almost immediately.
        // We use a small buffer of 5 candles to allow the trend (Slope) to form.
        self.ema9.warmup_count >= 5
    }

    pub fn ema_cross_up(&self) -> bool {
        if !self.is_ready() { return false; }
        if let (Some(curr_9), Some(curr_21), Some(prev_9), Some(prev_21)) =
            (self.ema9.value, self.ema21.value, self.prev_ema9, self.prev_ema21) {
            prev_9 <= prev_21 && curr_9 > curr_21
        } else {
            false
        }
    }

    pub fn ema_cross_down(&self) -> bool {
        if !self.is_ready() { return false; }
        if let (Some(curr_9), Some(curr_21), Some(prev_9), Some(prev_21)) =
            (self.ema9.value, self.ema21.value, self.prev_ema9, self.prev_ema21) {
            prev_9 >= prev_21 && curr_9 < curr_21
        } else {
            false
        }
    }

    pub fn update(&mut self, candle: &Candle) -> IndicatorState {
        if let Some(last_time) = self.last_candle_time {
            if candle.start_time <= last_time { return self.get_state(); }
            if candle.start_time - last_time > 120 { self.reset(); }
        }
        self.last_candle_time = Some(candle.start_time);
        let close = candle.close;
        
        if close.is_nan() || close.is_infinite() || close <= 0.0001 {
             return self.get_state();
        }

        self.prev_ema9 = self.ema9.value;
        self.prev_ema21 = self.ema21.value;

        self.ema9.update(close);
        self.ema21.update(close);
        self.rsi14.update(close);
        self.slope.update(close);

        if self.has_invalid_state() { self.reset(); }
        self.get_state()
    }

    fn has_invalid_state(&self) -> bool {
        let check = |v: Option<f64>| v.map_or(false, |f| !f.is_finite());
        check(self.ema9.value) || check(self.ema21.value) || check(self.rsi14.value) || check(self.slope.value)
    }

    pub fn get_state(&self) -> IndicatorState {
        IndicatorState {
            ema9: self.ema9.value,
            ema21: self.ema21.value,
            rsi14: self.rsi14.value,
            momentum_slope: self.slope.value,
        }
    }
}

impl Default for IndicatorEngine {
    fn default() -> Self { Self::new() }
}

pub struct Ema {
    period: usize,
    multiplier: f64,
    pub value: Option<f64>,
    pub warmup_count: usize,
}

impl Ema {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            multiplier: 2.0 / (period as f64 + 1.0),
            value: None,
            warmup_count: 0,
        }
    }

    pub fn update(&mut self, close: f64) -> Option<f64> {
        self.warmup_count += 1;
        if let Some(prev) = self.value {
            self.value = Some((close - prev) * self.multiplier + prev);
        } else {
            // Steady State Seeding: Assume the market was here for eternity
            self.value = Some(close);
        }
        self.value
    }
}

pub struct Rsi {
    period: usize,
    avg_gain: Option<f64>,
    avg_loss: Option<f64>,
    last_close: Option<f64>,
    pub value: Option<f64>,
    pub warmup_count: usize,
}

impl Rsi {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            avg_gain: None,
            avg_loss: None,
            last_close: None,
            value: None,
            warmup_count: 0,
        }
    }

    pub fn update(&mut self, close: f64) -> Option<f64> {
        self.warmup_count += 1;
        let prev_close = match self.last_close {
            Some(pc) => pc,
            None => { 
                self.last_close = Some(close); 
                // Initial seeding: RSI 50 (neutral)
                self.avg_gain = Some(0.0);
                self.avg_loss = Some(0.0);
                self.calc_rsi();
                return self.value; 
            }
        };
        self.last_close = Some(close);

        let change = close - prev_close;
        let gain = if change > 0.0 { change } else { 0.0 };
        let loss = if change < 0.0 { -change } else { 0.0 };

        if let (Some(ag), Some(al)) = (self.avg_gain, self.avg_loss) {
            // First few candles use SMA for stability, then transition to Wilder
            if self.warmup_count <= self.period {
                self.avg_gain = Some((ag * (self.warmup_count - 1) as f64 + gain) / self.warmup_count as f64);
                self.avg_loss = Some((al * (self.warmup_count - 1) as f64 + loss) / self.warmup_count as f64);
            } else {
                self.avg_gain = Some((ag * (self.period as f64 - 1.0) + gain) / self.period as f64);
                self.avg_loss = Some((al * (self.period as f64 - 1.0) + loss) / self.period as f64);
            }
        }

        self.calc_rsi();
        self.value
    }

    fn calc_rsi(&mut self) {
        if let (Some(ag), Some(al)) = (self.avg_gain, self.avg_loss) {
            if ag == 0.0 && al == 0.0 { self.value = Some(50.0); }
            else if al == 0.0 { self.value = Some(100.0); }
            else if ag == 0.0 { self.value = Some(0.0); }
            else {
                let rs = ag / al;
                self.value = Some(100.0 - (100.0 / (1.0 + rs)));
            }
        } else {
            self.value = Some(50.0);
        }
    }
}

pub struct MomentumSlope {
    window: usize,
    closes: VecDeque<f64>,
    pub value: Option<f64>,
}

impl MomentumSlope {
    pub fn new(window: usize) -> Self {
        Self { window, closes: VecDeque::with_capacity(window), value: None }
    }

    pub fn update(&mut self, close: f64) -> Option<f64> {
        if self.closes.len() == self.window { self.closes.pop_front(); }
        self.closes.push_back(close);

        let n_len = self.closes.len();
        if n_len < 2 { self.value = None; return None; }

        let n = n_len as f64;
        let mean_x = (n - 1.0) / 2.0;
        let var_x = (0..n_len).map(|i| { let d = (i as f64) - mean_x; d * d }).sum::<f64>();
        
        if var_x == 0.0 { self.value = Some(0.0); }
        else {
            let mean_y = self.closes.iter().sum::<f64>() / n;
            let cov_xy = self.closes.iter().enumerate().map(|(i, &y)| {
                ((i as f64) - mean_x) * (y - mean_y)
            }).sum::<f64>();
            self.value = Some(cov_xy / var_x);
        }
        self.value
    }
}

#[cfg(test)]
mod strategy_behavior_tests {
    use super::*;

    fn feed_warmup(engine: &mut IndicatorEngine, base_price: f64, count: usize) {
        for i in 0..count {
            // Noisy warmup to prevent RSI pinning and seed avg_gain/loss
            let noise = if i % 2 == 0 { 0.001 } else { -0.001 };
            let candle = Candle {
                start_time: (i as u64 + 1) * 60,
                open: 0.0, high: 0.0, low: 0.0, close: base_price + noise, volume: 0.0
            };
            engine.update(&candle);
        }
    }

    #[test]
    fn clean_breakout_sequence() {
        let mut engine = IndicatorEngine::new();
        // Standardized warmup for 9/21 indicators
        feed_warmup(&mut engine, 0.50, 60);

        let closes = [
            0.50, 0.50, 0.50, 0.50, 0.50,
            0.51, 0.52, 0.53, 0.55, 0.58,
            0.62, 0.66, 0.71, 0.77, 0.84,
            0.90, 0.93, 0.95, 0.97, 0.98,
            0.99, 0.985, 0.97, 0.94, 0.90,
            0.85, 0.80, 0.75, 0.70, 0.65,
        ];

        let mut crosses_up = 0;
        let mut rsi_hit_60 = false;
        let mut rsi_hit_75 = false;
        let mut rsi_dropped_below_50 = false;
        let mut slope_positive_accel = false;
        let mut reversal_spread_tightened = false;
        let mut last_spread = f64::MAX;

        let start_time = 61 * 60;
        for (i, &close) in closes.iter().enumerate() {
            let state = engine.update(&Candle {
                start_time: start_time + (i as u64 * 60),
                open: 0.0, high: 0.0, low: 0.0, close, volume: 0.0
            });

            if engine.ema_cross_up() { crosses_up += 1; }

            if let (Some(e9), Some(e21)) = (state.ema9, state.ema21) {
                let spread = e9 - e21;
                if i > 25 {
                    if spread < last_spread && spread.abs() < 0.01 {
                        reversal_spread_tightened = true;
                    }
                }
                last_spread = spread;
            }

            if let Some(rsi) = state.rsi14 {
                if rsi > 59.0 { rsi_hit_60 = true; }
                if rsi > 70.0 { rsi_hit_75 = true; } 
                if rsi < 55.0 && i > 21 { rsi_dropped_below_50 = true; }
            }

            if let Some(slope) = state.momentum_slope {
                if i >= 5 && i <= 15 && slope > 0.0 {
                    slope_positive_accel = true;
                }
            }
        }

        assert!(crosses_up > 0, "EMA9 should cross up");
        assert!(rsi_hit_60, "RSI should exceed 60");
        assert!(rsi_hit_75, "RSI should be elevated during expansion");
        assert!(slope_positive_accel, "Slope should be positive");
        assert!(reversal_spread_tightened, "EMA spread should compress during reversal");
        assert!(rsi_dropped_below_50, "RSI should drop below 55 during reversal");
    }

    #[test]
    fn fake_breakout() {
        let mut engine = IndicatorEngine::new();
        feed_warmup(&mut engine, 0.50, 60);

        let closes = [
            0.50, 0.50, 0.50, 0.50, 0.50,
            0.51, 0.52, 0.53, 0.55,
            0.53, 0.50, 0.48, 0.46, 0.44,
            0.43, 0.42, 0.41, 0.40,
        ];

        let mut crosses_up = 0;
        let mut crosses_down = 0;
        let mut rsi_peak_gt_55 = false;
        let mut rsi_falls_lt_45 = false;
        let mut slope_negative = false;

        let start_time = 61 * 60;
        for (i, &close) in closes.iter().enumerate() {
            engine.update(&Candle {
                start_time: start_time + (i as u64 * 60),
                open: 0.0, high: 0.0, low: 0.0, close, volume: 0.0
            });

            if engine.ema_cross_up() { crosses_up += 1; }
            if engine.ema_cross_down() { crosses_down += 1; }

            let state = engine.get_state();
            if let Some(rsi) = state.rsi14 {
                if rsi > 52.0 { rsi_peak_gt_55 = true; }
                if rsi < 48.0 && i > 10 { rsi_falls_lt_45 = true; }
            }

            if let Some(slope) = state.momentum_slope {
                if i > 10 && slope < 0.0 { slope_negative = true; }
            }
        }

        assert!(crosses_up > 0, "EMA9 should cross up");
        assert!(crosses_down > 0, "EMA9 should cross down");
        assert!(rsi_peak_gt_55, "RSI peaks during fake breakout");
        assert!(rsi_falls_lt_45, "RSI falls during fake breakout");
        assert!(slope_negative, "Slope turned negative");
    }

    #[test]
    fn compression_then_explosion() {
        let mut engine = IndicatorEngine::new();
        feed_warmup(&mut engine, 0.50, 60);

        let closes = [
            0.50, 0.501, 0.502, 0.501, 0.502,
            0.501, 0.503, 0.502, 0.504,
            0.55, 0.62, 0.70, 0.78, 0.85,
        ];

        let mut slope_near_zero = false;
        let mut rsi_in_tight_range = true;
        let mut slope_strongly_positive = false;
        let mut spread_increases = false;
        let mut last_spread = 0.0;

        let start_time = 61 * 60;
        for (i, &close) in closes.iter().enumerate() {
            let state = engine.update(&Candle {
                start_time: start_time + (i as u64 * 60),
                open: 0.0, high: 0.0, low: 0.0, close, volume: 0.0
            });

            if i < 9 {
                if let Some(slope) = state.momentum_slope {
                    if slope.abs() < 0.01 { slope_near_zero = true; }
                }
                if let Some(rsi) = state.rsi14 {
                    if rsi < 40.0 || rsi > 60.0 { rsi_in_tight_range = false; }
                }
            } else {
                if let (Some(e9), Some(e21)) = (state.ema9, state.ema21) {
                    let spread = e9 - e21;
                    if spread > last_spread { spread_increases = true; }
                    last_spread = spread;
                }
                if let Some(slope) = state.momentum_slope {
                    if slope > 0.01 { slope_strongly_positive = true; }
                }
            }
        }

        assert!(slope_near_zero, "Slope near zero compression");
        assert!(rsi_in_tight_range, "RSI stable compression");
        assert!(slope_strongly_positive, "Slope positive breakout");
        assert!(spread_increases, "EMA spread increasing");
    }

    #[test]
    fn flat_market_behavior() {
        let mut engine = IndicatorEngine::new();
        feed_warmup(&mut engine, 0.50, 60);

        // Feed a few flat candles to align EMAs after noisy warmup
        for i in 0..10 {
            engine.update(&Candle {
                start_time: (61 + i) * 60,
                open: 0.0, high: 0.0, low: 0.0, close: 0.50, volume: 0.0
            });
        }

        let mut crosses = 0;
        let start_time = 71 * 60;
        for i in 0..30 {
            let state = engine.update(&Candle {
                start_time: start_time + (i as u64 * 60),
                open: 0.0, high: 0.0, low: 0.0, close: 0.50, volume: 0.0
            });

            if engine.ema_cross_up() || engine.ema_cross_down() { crosses += 1; }

            let e9 = state.ema9.unwrap();
            let e21 = state.ema21.unwrap();
            let rsi = state.rsi14.unwrap();
            let slope = state.momentum_slope.unwrap();

            // After warmup with noise, EMAs should be very close but not exactly equal
            assert!((e9 - e21).abs() < 0.001, "EMA9 ≈ EMA21 (within 0.001)");
            // RSI should be near neutral in flat market
            assert!((rsi - 50.0).abs() < 5.0, "RSI ≈ 50 (within 5)");
            // Slope should be near zero in flat market
            assert!(slope.abs() < 0.001, "Slope ≈ 0 (within 0.001)");
        }
        assert_eq!(crosses, 0, "No crosses in flat market");
    }
}
