//! Volatility Calculator
//!
//! Realized volatility calculation for pricing models.

use std::collections::VecDeque;

/// Volatility surface with ATM skew support
#[derive(Debug, Clone, Default)]
pub struct VolSurface {
    pub atm_vol: f64,
    pub skew: f64,
    pub timestamp: i64,
}

impl VolSurface {
    pub fn new(atm_vol: f64, skew: f64, timestamp: i64) -> Self {
        Self { atm_vol, skew, timestamp }
    }

    /// Get vol at given moneyness
    pub fn vol_at_moneyness(&self, m: f64) -> f64 {
        // m = spot/strike, skew adjusts vol away from ATM
        let log_m = m.ln();
        self.atm_vol + self.skew * log_m
    }
}

/// Realized volatility calculator
pub struct VolatilityCalculator {
    /// 1-minute window
    window_1m: VecDeque<f64>,
    /// 5-minute window
    window_5m: VecDeque<f64>,
    /// 15-minute window
    window_15m: VecDeque<f64>,

    /// Last price
    last_price: Option<f64>,
    /// Log returns buffer
    returns: VecDeque<f64>,

    /// Max window size (in samples)
    max_samples: usize,
}

impl VolatilityCalculator {
    pub fn new() -> Self {
        Self {
            window_1m: VecDeque::with_capacity(60),
            window_5m: VecDeque::with_capacity(300),
            window_15m: VecDeque::with_capacity(900),
            last_price: None,
            returns: VecDeque::with_capacity(900),
            max_samples: 900,
        }
    }

    /// Add a price observation
    pub fn update(&mut self, price: f64) {
        // Calculate log return
        if let Some(last) = self.last_price {
            let ret = (price / last).ln();
            self.returns.push_back(ret);
            if self.returns.len() > self.max_samples {
                self.returns.pop_front();
            }
        }
        self.last_price = Some(price);
    }

    /// Calculate realized volatility for a given window
    ///
    /// Returns annualized volatility
    pub fn realized_vol(&self, window_seconds: i64) -> Option<f64> {
        let samples = (window_seconds as usize).min(self.returns.len());
        if samples < 10 {
            return None;
        }

        // Get returns for window
        let start = self.returns.len().saturating_sub(samples);
        let window: Vec<f64> = self.returns.iter().skip(start).copied().collect();

        // Calculate mean
        let mean = window.iter().sum::<f64>() / window.len() as f64;

        // Calculate variance
        let variance: f64 = window.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / window.len() as f64;

        // Annualize (assuming 1-second samples)
        let seconds_per_year = 365.25 * 24.0 * 3600.0;
        let annualized = (variance * seconds_per_year).sqrt();

        Some(annualized)
    }

    /// Get 1-minute realized vol
    pub fn vol_1m(&self) -> Option<f64> {
        self.realized_vol(60)
    }

    /// Get 5-minute realized vol
    pub fn vol_5m(&self) -> Option<f64> {
        self.realized_vol(300)
    }

    /// Get 15-minute realized vol
    pub fn vol_15m(&self) -> Option<f64> {
        self.realized_vol(900)
    }

    /// Reset calculator
    pub fn reset(&mut self) {
        self.window_1m.clear();
        self.window_5m.clear();
        self.window_15m.clear();
        self.last_price = None;
        self.returns.clear();
    }

    /// Build volatility surface from current state
    pub fn build_surface(&self, timestamp: i64) -> Option<VolSurface> {
        let atm_vol = self.vol_5m()?;
        // Simple skew estimation - could be enhanced
        let skew = 0.1; // Placeholder
        Some(VolSurface::new(atm_vol, skew, timestamp))
    }
}

impl Default for VolatilityCalculator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_calculator_returns_none() {
        let calc = VolatilityCalculator::new();
        assert!(calc.vol_1m().is_none());
    }

    #[test]
    fn calculates_vol_after_updates() {
        let mut calc = VolatilityCalculator::new();

        // Add 100 samples with small variation
        for i in 0..100 {
            let price = 100.0 + (i as f64 * 0.01).sin() * 0.5;
            calc.update(price);
        }

        let vol = calc.vol_1m();
        assert!(vol.is_some());
        let vol = vol.unwrap();
        assert!(vol > 0.0 && vol < 2.0);
    }

    #[test]
    fn vol_surface_atm() {
        let surface = VolSurface::new(0.60, 0.1, 1000);
        assert!((surface.vol_at_moneyness(1.0) - 0.60).abs() < 1e-6);
    }
}
