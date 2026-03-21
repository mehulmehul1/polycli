//! Fair Value Model
//!
//! Digital option approximation for crypto binary markets.

use serde::Deserialize;

/// Fair value model configuration
#[derive(Debug, Clone, Deserialize)]
pub struct FairValueConfig {
    /// Annualized volatility estimate
    pub base_volatility: f64,
    /// Vol scaling for short horizons
    pub vol_scale_short: f64,
    /// Drift adjustment weight
    pub drift_weight: f64,
    /// Mean reversion for extreme prices
    pub mean_reversion_strength: f64,
    /// Use realized vol instead of base
    pub use_realized_vol: bool,
    /// Realized vol lookback in seconds
    pub realized_vol_lookback: i64,
}

impl Default for FairValueConfig {
    fn default() -> Self {
        Self {
            base_volatility: 0.60,
            vol_scale_short: 1.2,
            drift_weight: 0.10,
            mean_reversion_strength: 0.05,
            use_realized_vol: true,
            realized_vol_lookback: 300,
        }
    }
}

/// Fair value model for crypto binary markets
pub struct FairValueModel {
    config: FairValueConfig,
}

impl FairValueModel {
    pub fn new(config: FairValueConfig) -> Self {
        Self { config }
    }

    /// Calculate fair probability for up-down market
    ///
    /// # Arguments
    /// * `spot_current` - Current spot price
    /// * `spot_at_open` - Spot price at market open
    /// * `time_remaining_s` - Seconds until market resolution
    /// * `realized_vol` - Recent realized volatility (if available)
    ///
    /// # Returns
    /// Fair probability of YES (market closes higher than open)
    pub fn fair_prob_updown(
        &self,
        spot_current: f64,
        spot_at_open: f64,
        time_remaining_s: i64,
        realized_vol: Option<f64>,
    ) -> f64 {
        // Convert to years
        let t = (time_remaining_s as f64).max(1.0) / (365.25 * 24.0 * 3600.0);

        // Use realized vol if available
        let sigma_ann =
            realized_vol.unwrap_or(self.config.base_volatility) * self.config.vol_scale_short;

        // Per-time volatility
        let sigma_t = sigma_ann * t.sqrt();

        // Log return since open
        let log_return = (spot_current / spot_at_open).ln();

        // Drift adjustment
        let drift = log_return * self.config.drift_weight;

        // d2 for up-down (strike = spot_at_open)
        let d2 = -sigma_t / 2.0 + drift;

        // Standard normal CDF
        normal_cdf(d2)
    }

    /// Calculate fair probability for threshold-at-expiry market
    ///
    /// # Arguments
    /// * `spot_current` - Current spot price
    /// * `strike` - Target price threshold
    /// * `time_remaining_s` - Seconds until market resolution
    /// * `realized_vol` - Recent realized volatility (if available)
    ///
    /// # Returns
    /// Fair probability of YES (spot above strike at expiry)
    pub fn fair_prob_threshold(
        &self,
        spot_current: f64,
        strike: f64,
        time_remaining_s: i64,
        realized_vol: Option<f64>,
    ) -> f64 {
        if time_remaining_s <= 0 {
            // Already expired - check current price
            return if spot_current > strike { 1.0 } else { 0.0 };
        }

        let t = (time_remaining_s as f64) / (365.25 * 24.0 * 3600.0);
        let sigma_ann =
            realized_vol.unwrap_or(self.config.base_volatility) * self.config.vol_scale_short;

        let sigma_t = sigma_ann * t.sqrt();

        // d2 = ln(S/K) / (sigma * sqrt(T)) - sigma * sqrt(T) / 2
        let m = (spot_current / strike).ln();
        let d2 = m / sigma_t - sigma_t / 2.0;

        normal_cdf(d2)
    }

    /// Calculate edge (fair_prob - market_price)
    pub fn edge(&self, fair_prob: f64, market_price: f64) -> f64 {
        fair_prob - market_price
    }

    /// Check if edge is significant enough for entry
    pub fn has_edge(&self, fair_prob: f64, market_price: f64, threshold: f64) -> bool {
        self.edge(fair_prob, market_price).abs() > threshold
    }

    /// Infer implied volatility from market price
    ///
    /// Uses Newton-Raphson iteration
    pub fn implied_vol_from_market(
        &self,
        spot_current: f64,
        strike: f64,
        time_remaining_s: i64,
        market_price: f64,
        max_iterations: usize,
    ) -> Option<f64> {
        if time_remaining_s <= 0 || market_price <= 0.0 || market_price >= 1.0 {
            return None;
        }

        let t = (time_remaining_s as f64) / (365.25 * 24.0 * 3600.0);
        let m = (spot_current / strike).ln();

        // Initial guess
        let mut sigma = self.config.base_volatility;

        for _ in 0..max_iterations {
            let sigma_t = sigma * t.sqrt();
            let d2 = m / sigma_t - sigma_t / 2.0;

            let price = normal_cdf(d2);
            let vega = normal_pdf(d2) * t.sqrt() / (sigma_t + 1e-10);

            let diff = price - market_price;

            if diff.abs() < 1e-6 {
                return Some(sigma);
            }

            // Newton-Raphson update
            sigma -= diff / (vega + 1e-10);
            sigma = sigma.clamp(0.01, 5.0);
        }

        Some(sigma)
    }
}

impl Default for FairValueModel {
    fn default() -> Self {
        Self::new(FairValueConfig::default())
    }
}

/// Standard normal CDF approximation
fn normal_cdf(x: f64) -> f64 {
    // Abramowitz and Stegun approximation
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let p = 0.3275911;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs() / 2.0_f64.sqrt();

    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();

    0.5 * (1.0 + sign * y)
}

/// Standard normal PDF
fn normal_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fair_prob_updown_roughly_50() {
        let model = FairValueModel::default();
        // At the money, no drift - should be close to 0.5
        let prob = model.fair_prob_updown(100.0, 100.0, 300, None);
        assert!(prob > 0.4 && prob < 0.6);
    }

    #[test]
    fn fair_prob_threshold_above_strike() {
        let model = FairValueModel::default();
        // Spot above strike - should be > 0.5
        let prob = model.fair_prob_threshold(105.0, 100.0, 300, None);
        assert!(prob > 0.5);
    }

    #[test]
    fn fair_prob_threshold_below_strike() {
        let model = FairValueModel::default();
        // Spot below strike - should be < 0.5
        let prob = model.fair_prob_threshold(95.0, 100.0, 300, None);
        assert!(prob < 0.5);
    }

    #[test]
    fn edge_calculation() {
        let model = FairValueModel::default();
        let edge = model.edge(0.55, 0.50);
        assert!((edge - 0.05).abs() < 1e-6);
    }

    #[test]
    fn implied_vol_round_trip() {
        let model = FairValueModel::default();
        let vol = 0.80;
        let price = model.fair_prob_threshold(105.0, 100.0, 300, Some(vol));
        let implied = model.implied_vol_from_market(105.0, 100.0, 300, price, 100);
        assert!(implied.is_some());
        let implied = implied.unwrap();
        assert!((implied - vol).abs() < 0.1);
    }
}
