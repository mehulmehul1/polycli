//! Calibrated Fair Value Model
//!
//! Microstructure adjustments to the theoretical Black-Scholes model.

use super::FairValueModel;
use serde::{Deserialize, Serialize};

/// Calibration configuration for fair value adjustments
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalibrationConfig {
    /// Spread adjustment factor (0.0 = ignore, 0.5 = half spread, 1.0 = full spread)
    pub spread_adjustment: f64,
    /// Book imbalance weight for probability adjustment
    pub book_imbalance_weight: f64,
    /// Historical bias adjustment (e.g., if YES wins 55% of time, add 0.025)
    pub historical_bias: f64,
    /// Minimum edge threshold for entry (in probability units)
    pub min_edge_threshold: f64,
    /// Maximum edge for confidence capping
    pub max_edge: f64,
    /// Minimum probability bound (avoid extremes)
    pub min_prob: f64,
    /// Maximum probability bound (avoid extremes)
    pub max_prob: f64,
}

impl Default for CalibrationConfig {
    fn default() -> Self {
        Self {
            spread_adjustment: 0.5,
            book_imbalance_weight: 0.10,
            historical_bias: 0.0,
            min_edge_threshold: 0.03,
            max_edge: 0.10,
            min_prob: 0.05,
            max_prob: 0.95,
        }
    }
}

/// Calibrated probability output
#[derive(Debug, Clone, Serialize)]
pub struct CalibratedProb {
    /// Base fair probability from Black-Scholes
    pub fair_prob_base: f64,
    /// Fair probability after spread adjustment
    pub fair_prob_adjusted: f64,
    /// Final calibrated probability after book imbalance
    pub fair_prob_calibrated: f64,
    /// Edge for YES (fair - market_ask)
    pub edge_yes: f64,
    /// Edge for NO ((1 - fair) - no_ask)
    pub edge_no: f64,
    /// Whether the edge is significant enough to trade
    pub tradeable: bool,
    /// Recommended direction (if tradeable)
    pub direction: Option<CalibratedDirection>,
}

/// Trade direction recommendation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CalibratedDirection {
    Yes,
    No,
}

/// Calibrated fair value model
pub struct CalibratedFairValue {
    base_model: FairValueModel,
    config: CalibrationConfig,
}

impl CalibratedFairValue {
    /// Create a new calibrated fair value model
    pub fn new(base_model: FairValueModel, config: CalibrationConfig) -> Self {
        Self { base_model, config }
    }

    /// Create with default config
    pub fn with_defaults(base_model: FairValueModel) -> Self {
        Self::new(base_model, CalibrationConfig::default())
    }

    /// Calculate calibrated fair value
    ///
    /// # Arguments
    /// * `spot` - Current spot price (or probability for up/down markets)
    /// * `spot_at_open` - Spot price at market open (reference for drift calculation)
    /// * `time_remaining_s` - Seconds until market resolution
    /// * `realized_vol` - Realized volatility (if available)
    /// * `yes_ask` - Current YES ask price
    /// * `no_ask` - Current NO ask price
    /// * `yes_spread` - YES spread (ask - bid)
    /// * `book_sum` - Sum of YES and NO prices (for inefficiency detection)
    /// * `min_edge_threshold` - Minimum edge for tradeability
    ///
    /// # Returns
    /// Calibrated probability with edge calculations
    pub fn calculate(
        &self,
        spot: f64,
        spot_at_open: f64,
        time_remaining_s: i64,
        realized_vol: Option<f64>,
        yes_ask: f64,
        no_ask: f64,
        yes_spread: f64,
        book_sum: f64,
        min_edge_threshold: f64,
    ) -> CalibratedProb {
        // Debug logging to verify fix
        if spot_at_open != 1.0 {
            println!(
                "[FAIRVALUE DEBUG] spot_at_open={:.4} spot={:.4} log_return={:.4}",
                spot_at_open,
                spot,
                (spot / spot_at_open).ln()
            );
        }

        // Calculate base fair probability using Black-Scholes
        let fair_prob_base =
            self.base_model
                .fair_prob_updown(spot, spot_at_open, time_remaining_s, realized_vol);

        // Clamp to valid range
        let fair_prob_base = fair_prob_base.clamp(self.config.min_prob, self.config.max_prob);

        // Adjust for spread cost
        // If we buy at ask, we pay spread - reduce fair prob by half spread
        let spread_cost = yes_spread * self.config.spread_adjustment;
        let fair_prob_adjusted = fair_prob_base - spread_cost;

        // Adjust for book imbalance
        // book_sum < 1.0 means YES is undervalued (both YES and NO are cheap)
        // book_sum > 1.0 means YES is overvalued
        let book_inefficiency = 1.0 - book_sum;
        let book_adjustment = book_inefficiency * self.config.book_imbalance_weight;

        // Apply historical bias (learned from past outcomes)
        let fair_prob_calibrated =
            (fair_prob_adjusted + book_adjustment + self.config.historical_bias)
                .clamp(self.config.min_prob, self.config.max_prob);

        // Calculate edges
        let edge_yes = fair_prob_calibrated - yes_ask;
        let edge_no = (1.0 - fair_prob_calibrated) - no_ask;

        // Determine if tradeable
        let tradeable = edge_yes.abs() >= min_edge_threshold || edge_no.abs() >= min_edge_threshold;

        // Determine direction based on which edge is larger
        let direction = if tradeable {
            if edge_yes.abs() >= edge_no.abs() {
                if edge_yes > 0.0 {
                    Some(CalibratedDirection::Yes)
                } else {
                    Some(CalibratedDirection::No)
                }
            } else {
                if edge_no > 0.0 {
                    Some(CalibratedDirection::No)
                } else {
                    Some(CalibratedDirection::Yes)
                }
            }
        } else {
            None
        };

        CalibratedProb {
            fair_prob_base,
            fair_prob_adjusted,
            fair_prob_calibrated,
            edge_yes,
            edge_no,
            tradeable,
            direction,
        }
    }

    /// Calculate confidence from edge
    pub fn confidence_from_edge(&self, edge: f64) -> f64 {
        (edge.abs() / self.config.max_edge).clamp(0.0, 1.0)
    }

    /// Update calibration config
    pub fn update_config(&mut self, config: CalibrationConfig) {
        self.config = config;
    }

    /// Get current config
    pub fn config(&self) -> &CalibrationConfig {
        &self.config
    }

    /// Get inner model
    pub fn model(&self) -> &FairValueModel {
        &self.base_model
    }
}

impl Default for CalibratedFairValue {
    fn default() -> Self {
        Self::with_defaults(FairValueModel::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_prob_within_bounds() {
        let model = CalibratedFairValue::default();

        let result = model.calculate(
            1.0,       // spot
            1.0,       // spot_at_open
            300,       // 5 minutes
            Some(0.6), // vol
            0.50,      // yes_ask
            0.50,      // no_ask
            0.02,      // spread
            1.0,       // book_sum
            0.03,      // min_edge_threshold
        );

        assert!(result.fair_prob_calibrated >= 0.05);
        assert!(result.fair_prob_calibrated <= 0.95);
    }

    #[test]
    fn spread_adjustment_reduces_fair_prob() {
        let model = CalibratedFairValue::default();

        let result_no_spread =
            model.calculate(1.0, 1.0, 300, Some(0.6), 0.50, 0.50, 0.0, 1.0, 0.03);
        let result_with_spread =
            model.calculate(1.0, 1.0, 300, Some(0.6), 0.50, 0.50, 0.04, 1.0, 0.03);

        // Spread adjustment should reduce fair probability
        assert!(result_with_spread.fair_prob_adjusted < result_no_spread.fair_prob_adjusted);
    }

    #[test]
    fn book_inefficiency_adjustment() {
        let model = CalibratedFairValue::default();

        // book_sum < 1.0 (YES undervalued) should increase fair prob
        let result_undervalued =
            model.calculate(1.0, 1.0, 300, Some(0.6), 0.45, 0.45, 0.02, 0.90, 0.03);

        // book_sum > 1.0 (YES overvalued) should decrease fair prob
        let result_overvalued =
            model.calculate(1.0, 1.0, 300, Some(0.6), 0.55, 0.55, 0.02, 1.10, 0.03);

        assert!(result_undervalued.fair_prob_calibrated > result_overvalued.fair_prob_calibrated);
    }

    #[test]
    fn tradeable_when_edge_exceeds_threshold() {
        let model = CalibratedFairValue::default();

        // Large mispricing should be tradeable
        let result = model.calculate(1.0, 1.0, 300, Some(0.6), 0.45, 0.55, 0.02, 1.0, 0.03);

        assert!(result.tradeable);
        assert!(result.direction.is_some());
    }

    #[test]
    fn not_tradeable_when_edge_below_threshold() {
        let model = CalibratedFairValue::default();

        // Small mispricing should not be tradeable
        let result = model.calculate(1.0, 1.0, 300, Some(0.6), 0.49, 0.51, 0.02, 1.0, 0.03);

        assert!(!result.tradeable);
    }

    #[test]
    fn confidence_from_edge() {
        let model = CalibratedFairValue::default();

        let conf_low = model.confidence_from_edge(0.01);
        let conf_mid = model.confidence_from_edge(0.05);
        let conf_high = model.confidence_from_edge(0.10);

        assert!(conf_low < conf_mid);
        assert!(conf_mid <= conf_high);
        assert!(conf_high <= 1.0);
    }

    #[test]
    fn direction_based_on_edge_sign() {
        let model = CalibratedFairValue::default();

        // YES edge positive -> YES direction
        let result_yes = model.calculate(1.0, 1.0, 300, Some(0.6), 0.40, 0.60, 0.02, 1.0, 0.03);
        assert_eq!(result_yes.direction, Some(CalibratedDirection::Yes));
        assert!(result_yes.edge_yes > 0.0);

        // NO edge positive -> NO direction
        let result_no = model.calculate(1.0, 1.0, 300, Some(0.6), 0.60, 0.40, 0.02, 1.0, 0.03);
        assert_eq!(result_no.direction, Some(CalibratedDirection::No));
        assert!(result_no.edge_no > 0.0);
    }

    #[test]
    fn historical_bias_adjusts_fair_prob() {
        let config_no_bias = CalibrationConfig {
            historical_bias: 0.0,
            ..Default::default()
        };
        let config_with_bias = CalibrationConfig {
            historical_bias: 0.05,
            ..Default::default()
        };

        let model_no_bias = CalibratedFairValue::new(FairValueModel::default(), config_no_bias);
        let model_with_bias = CalibratedFairValue::new(FairValueModel::default(), config_with_bias);

        let result_no_bias =
            model_no_bias.calculate(1.0, 1.0, 300, Some(0.6), 0.50, 0.50, 0.02, 1.0, 0.03);
        let result_with_bias =
            model_with_bias.calculate(1.0, 1.0, 300, Some(0.6), 0.50, 0.50, 0.02, 1.0, 0.03);

        // Positive bias should increase calibrated probability
        assert!(result_with_bias.fair_prob_calibrated > result_no_bias.fair_prob_calibrated);
    }
}
