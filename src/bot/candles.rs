use std::collections::VecDeque;

const MAX_BUFFER_LEN: usize = 100;

#[derive(Clone, Copy, Debug)]
pub struct Candle {
    pub start_time: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VolumeMode {
    #[default]
    Snapshot,
    #[allow(dead_code)]
    Delta,
}

pub struct CandleEngine {
    five_second: CandleAggregator,
    fifteen_second: CandleAggregator,
    one_minute: CandleAggregator,
}

impl CandleEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            five_second: CandleAggregator::new(5, MAX_BUFFER_LEN, VolumeMode::Snapshot, false),
            fifteen_second: CandleAggregator::new(15, MAX_BUFFER_LEN, VolumeMode::Snapshot, false),
            one_minute: CandleAggregator::new(60, MAX_BUFFER_LEN, VolumeMode::Snapshot, false),
        }
    }

    pub fn set_debug(&mut self, enabled: bool) {
        self.five_second.debug_logs = enabled;
        self.fifteen_second.debug_logs = enabled;
        self.one_minute.debug_logs = enabled;
    }

    pub fn set_volume_mode(&mut self, mode: VolumeMode) {
        self.five_second.volume_mode = mode;
        self.fifteen_second.volume_mode = mode;
        self.one_minute.volume_mode = mode;
    }

    /// Update with a strict epoch aligned timestamp. Returns true if price was accepted.
    pub fn update(&mut self, price: f64, spread: f64, volume: f64, epoch_seconds: u64) -> bool {
        if !is_price_valid(price, spread) {
            return false;
        }

        self.five_second.update(price, volume, epoch_seconds);
        self.fifteen_second.update(price, volume, epoch_seconds);
        self.one_minute.update(price, volume, epoch_seconds);

        true
    }

    #[must_use]
    pub fn get_last_5s(&self) -> Option<Candle> {
        self.five_second.last()
    }

    #[must_use]
    pub fn get_last_15s(&self) -> Option<Candle> {
        self.fifteen_second.last()
    }

    #[must_use]
    pub fn get_last_1m(&self) -> Option<Candle> {
        self.one_minute.last()
    }
}

impl Default for CandleEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CandleAggregator {
    interval_seconds: u64,
    current: Option<Candle>,
    buffer: VecDeque<Candle>,
    max_len: usize,
    volume_mode: VolumeMode,
    last_snapshot_vol: Option<f64>,
    debug_logs: bool,
}

impl CandleAggregator {
    pub fn new(interval_seconds: u64, max_len: usize, volume_mode: VolumeMode, debug_logs: bool) -> Self {
        Self {
            interval_seconds,
            current: None,
            buffer: VecDeque::with_capacity(max_len),
            max_len,
            volume_mode,
            last_snapshot_vol: None,
            debug_logs,
        }
    }

    pub fn update(&mut self, price: f64, volume: f64, epoch_seconds: u64) {
        let bucket_start = (epoch_seconds / self.interval_seconds) * self.interval_seconds;

        let delta_vol = match self.volume_mode {
            VolumeMode::Delta => volume,
            VolumeMode::Snapshot => {
                let prev = self.last_snapshot_vol.unwrap_or(volume);
                self.last_snapshot_vol = Some(volume);
                (volume - prev).max(0.0) // prevent negative volume spikes if depth drops dramatically
            }
        };

        if let Some(current) = self.current.as_mut() {
            if bucket_start == current.start_time {
                current.high = current.high.max(price);
                current.low = current.low.min(price);
                current.close = price;
                current.volume += delta_vol;
                return;
            }

            if bucket_start < current.start_time {
                // Ignore late/out-of-order ticks
                return;
            }

            if bucket_start > current.start_time {
                // Close current candle and push to buffer
                let closed = self.current.take().unwrap();
                if self.debug_logs {
                    println!(
                        "[CANDLE CLOSE {}s] O={:.4} H={:.4} L={:.4} C={:.4} V={:.4}",
                        self.interval_seconds,
                        closed.open, closed.high, closed.low, closed.close, closed.volume
                    );
                }
                self.push(closed);
            }
        }

        // Initialize new candle
        if self.debug_logs {
            println!(
                "[CANDLE START {}s] open={:.4} start={}",
                self.interval_seconds, price, bucket_start
            );
        }

        self.current = Some(Candle {
            start_time: bucket_start,
            open: price,
            high: price,
            low: price,
            close: price,
            // For snapshot mode, the first tick contributes delta 0 usually, but we record the delta anyway.
            volume: delta_vol,
        });
    }

    fn push(&mut self, candle: Candle) {
        if self.buffer.len() == self.max_len {
            self.buffer.pop_front();
        }
        self.buffer.push_back(candle);
    }

    pub fn last(&self) -> Option<Candle> {
        self.buffer.back().copied()
    }
}

fn is_price_valid(price: f64, spread: f64) -> bool {
    if !price.is_finite() {
        return false;
    }
    if price <= 0.01 {
        return false;
    }
    if price >= 0.99 && spread > 0.9 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_and_rolls_5s_candles() {
        let mut engine = CandleEngine::new();
        let t0 = 100_000_u64; // nice even multiple of 60

        for second in 0..5_u64 {
            engine.update(
                100.0 + (second as f64),
                0.05,
                1.0, 
                t0 + second,
            );
        }

        assert!(engine.get_last_5s().is_none());

        engine.update(200.0, 0.05, 2.0, t0 + 5);
        let first = engine
            .get_last_5s()
            .expect("expected first closed 5s candle");

        assert_eq!(first.start_time, 100_000);
        assert_eq!(first.open, 100.0);
        assert_eq!(first.high, 104.0);
        assert_eq!(first.low, 100.0);
        assert_eq!(first.close, 104.0);
    }

    #[test]
    fn invalid_prices_ignored() {
        let mut engine = CandleEngine::new();
        let t0 = 100_000_u64;

        assert!(!engine.update(0.005, 0.0, 1.0, t0)); // <= 0.01
        assert!(!engine.update(0.995, 0.95, 1.0, t0)); // > 0.99 & spread > 0.9
        assert!(!engine.update(f64::NAN, 0.1, 1.0, t0)); // NaN
        assert!(!engine.update(f64::INFINITY, 0.1, 1.0, t0)); // Inf

        engine.update(0.50, 0.1, 1.0, t0); // valid

        engine.update(0.51, 0.1, 1.0, t0 + 5); // roll candle
        let last = engine.get_last_5s().unwrap();
        assert_eq!(last.open, 0.50);
    }

    #[test]
    fn specific_65s_boundary_test() {
        // Prices: 0.70, 0.71, 0.69, 0.72 across 65 seconds
        let mut engine = CandleEngine::new();
        // Start at an exact minute boundary to simplify 1m test checks
        let start_ts = 1_700_000_400_u64; // multiple of 60

        engine.update(0.70, 0.02, 100.0, start_ts);
        engine.update(0.71, 0.02, 110.0, start_ts + 10);
        engine.update(0.69, 0.02, 120.0, start_ts + 35);
        engine.update(0.72, 0.02, 130.0, start_ts + 65);

        let c5 = engine.get_last_5s().unwrap();
        // 65s crossed multiple 5s bounds. Last closed 5s bucket is start_ts + 35.
        assert_eq!(c5.start_time, start_ts + 35);
        assert_eq!(c5.open, 0.69);

        let c15 = engine.get_last_15s().unwrap();
        // 65s crossed multiple 15 bounds. Last closed 15s bucket containing a tick is start_ts + 30
        assert_eq!(c15.start_time, start_ts + 30);
        assert_eq!(c15.open, 0.69);

        let c1m = engine.get_last_1m().unwrap();
        // 1m bucket started at `start_ts`. Crossed when we hit `start_ts + 65`.
        assert_eq!(c1m.start_time, start_ts);
        assert_eq!(c1m.open, 0.70); // Must be first valid price
        assert_eq!(c1m.high, 0.71);
        assert_eq!(c1m.low, 0.69);
        assert_eq!(c1m.close, 0.69);

        // Volume check (Snapshot mode: 10 + 10 + 10 = 30 delta total)
        assert_eq!(c1m.volume, 30.0);
        }
}
