//! Probability Engine for Temporal Arbitrage
//!
//! This module implements probability calculations for binary options
//! using Black-Scholes-style models adapted for prediction markets.
//!
//! Key features:
//! - Standard Black-Scholes binary option probability
//! - Chained conditional probability after observing some legs
//! - Implied parent probability from partial observations

use serde::{Deserialize, Serialize};
use std::fmt;

/// Violation type for constraint detection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViolationType {
    /// Chained conditional arbitrage
    ChainedConditional,
    /// Late-phase pricing anomaly
    LatePhaseAnomaly,
    /// Price consistency violation
    PriceConsistency,
}

impl fmt::Display for ViolationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViolationType::ChainedConditional => write!(f, "ChainedConditional"),
            ViolationType::LatePhaseAnomaly => write!(f, "LatePhaseAnomaly"),
            ViolationType::PriceConsistency => write!(f, "PriceConsistency"),
        }
    }
}

/// Arbitrage action to take
#[derive(Debug, Clone)]
pub enum ArbitrageAction {
    /// Enter a single market position
    EnterSingle {
        condition_id: String,
        direction: super::Direction,
        edge: f64,
        reason: ArbitrageActionReason,
    },
    /// Hold - no action
    Hold,
}

/// Reason for the arbitrage action
#[derive(Debug, Clone)]
pub enum ArbitrageActionReason {
    ChainedConditional(f64),
    LatePhaseAnomaly(f64),
    PriceConsistency(f64),
}

impl std::fmt::Display for ArbitrageActionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArbitrageActionReason::ChainedConditional(c) => {
                write!(f, "ChainedConditional(conf={:.2})", c)
            }
            ArbitrageActionReason::LatePhaseAnomaly(c) => {
                write!(f, "LatePhaseAnomaly(conf={:.2})", c)
            }
            ArbitrageActionReason::PriceConsistency(c) => {
                write!(f, "PriceConsistency(conf={:.2})", c)
            }
        }
    }
}

/// Probability calculation engine
pub struct ProbabilityEngine;

impl ProbabilityEngine {
    /// Standard Black-Scholes binary option probability
    ///
    /// Calculates the probability that price > strike at expiration.
    ///
    /// # Arguments
    /// * `current_price` - Current asset price
    /// * `strike_price` - Strike price for the binary option
    /// * `time_remaining_sec` - Time until expiration in seconds
    /// * `vol_per_sec` - Volatility per second (annualized / sqrt(seconds_per_year))
    /// * `drift` - Risk-neutral drift rate (optional, defaults to 0)
    ///
    /// # Returns
    /// Probability in range [0, 1]
    ///
    /// # Formula
    /// ```text
    /// P(up) = Φ((S - K + μ*τ) / (σ*√τ))
    ///
    /// where:
    /// - S = current price
    /// - K = strike price
    /// - τ = time remaining
    /// - σ = volatility per second
    /// - μ = drift rate
    /// - Φ = standard normal CDF
    /// ```
    pub fn prob_up(
        current_price: f64,
        strike_price: f64,
        time_remaining_sec: f64,
        vol_per_sec: f64,
        drift: f64,
    ) -> f64 {
        let distance = current_price - strike_price;

        // If already above strike, high probability with small reversal risk
        if distance >= 0.0 {
            let reversal_prob = 0.02 * (time_remaining_sec / 3600.0).min(1.0);
            return 1.0 - reversal_prob;
        }

        // Calculate standard deviation
        let std = vol_per_sec * time_remaining_sec.sqrt();
        if std < 1e-6 {
            return 0.5;
        }

        // Apply drift adjustment
        let drift_adjustment = drift * time_remaining_sec;
        let z = (distance + drift_adjustment) / std;

        norm_cdf(z)
    }

    /// Chained conditional probability after observing some legs
    ///
    /// CORRECTED: After k of n children resolve, we observe the actual price path.
    /// The parent's fair probability depends on the LIVE BTC price, not the price
    /// after the last leg.
    ///
    /// # Key Insight
    /// Each child market has a **resetting strike** (the open of its own interval).
    /// After observing some legs, we have partial information about the price path,
    /// but the parent's resolution depends on whether the FINAL price exceeds the
    /// ORIGINAL strike (first child's open price).
    ///
    /// # Arguments
    /// * `observed_legs` - (close_price, outcome) for each resolved leg
    /// * `original_strike` - Parent's strike (P₀) - first child's close
    /// * `live_btc_price` - ALWAYS use fresh live spot price
    /// * `remaining_time_sec` - Time until parent resolves
    /// * `vol_per_sec` - Volatility per second
    /// * `drift` - Risk-neutral drift rate
    ///
    /// # Formula
    /// ```text
    /// P(parent=YES | observed path) = Φ((P_live - P₀) / (σ * √t_remaining))
    /// ```
    ///
    /// # Example
    /// ```text
    /// 15min market (1:15-1:30PM), strike = $71,496:
    ///   ├─ 5m #1 (1:15-1:20): close = $71,550 (YES, beat P₀)
    ///   ├─ 5m #2 (1:20-1:25): close = $71,530 (NO, didn't beat P₁=$71,550)
    ///   └─ 5m #3 (1:25-1:30): pending
    ///
    /// Live BTC: $71,530
    /// Need: P₃ > P₀ = $71,496
    ///
    /// P(parent=YES) = Φ((71530 - 71496) / (σ * √120)) ≈ 0.60
    /// ```
    pub fn chained_conditional_after_observed_legs(
        observed_legs: &[(f64, bool)], // (close_price, outcome) for each resolved leg
        original_strike: f64,          // Parent's strike (P₀) - first child's close
        live_btc_price: f64,           // FIX: Always use fresh live spot
        remaining_time_sec: f64,
        vol_per_sec: f64,
        drift: f64,
    ) -> f64 {
        // If no legs observed, fall back to base probability
        if observed_legs.is_empty() {
            return Self::prob_up(
                live_btc_price,
                original_strike,
                remaining_time_sec,
                vol_per_sec,
                drift,
            );
        }

        // FIX: Use live BTC price from graph state
        // The observed legs confirm the path reached this point, but we always
        // use the freshest spot price available

        let distance = live_btc_price - original_strike;

        // If currently above original strike, high confidence
        if distance >= 0.0 {
            return 0.98; // High confidence, but small reversal risk
        }

        // Calculate probability using Black-Scholes
        let std = vol_per_sec * remaining_time_sec.sqrt();
        if std < 1e-6 {
            return 0.5;
        }

        let drift_adjustment = drift * remaining_time_sec;
        let z = (distance + drift_adjustment) / std;
        norm_cdf(z)
    }

    /// Estimate probability from partial observations
    ///
    /// After k of n legs resolved, blend the base model with empirical signal
    /// from the resolved legs.
    ///
    /// # Arguments
    /// * `k_yes` - Number of legs that resolved YES
    /// * `k_total` - Total number of legs resolved
    /// * `live_price` - Current live BTC price
    /// * `original_strike` - Parent's original strike price
    /// * `remaining_time_sec` - Time until parent resolves
    /// * `vol_per_sec` - Volatility per second
    ///
    /// # Returns
    /// Blended probability estimate
    pub fn implied_parent_prob_after_k_legs(
        k_yes: usize,
        k_total: usize,
        live_price: f64,
        original_strike: f64,
        remaining_time_sec: f64,
        vol_per_sec: f64,
    ) -> f64 {
        if k_total == 0 {
            return 0.5;
        }

        // Base probability from Black-Scholes
        let base_prob = Self::prob_up(
            live_price,
            original_strike,
            remaining_time_sec,
            vol_per_sec,
            0.0,
        );

        // Empirical signal from resolved legs
        let yes_fraction = k_yes as f64 / k_total as f64;

        // Blend: 70% base model, 30% empirical signal
        // This weights the theoretical model more heavily while still
        // incorporating the information from resolved legs
        let blended = base_prob * 0.7 + yes_fraction * 0.3;

        blended.clamp(0.02, 0.98)
    }

    /// Calculate probability with confidence interval
    ///
    /// # Arguments
    /// * `current_price` - Current asset price
    /// * `strike_price` - Strike price
    /// * `time_remaining_sec` - Time until expiration
    /// * `vol_per_sec` - Volatility per second
    /// * `confidence_level` - Z-score for confidence interval (1.96 for 95%)
    ///
    /// # Returns
    /// (probability, lower_bound, upper_bound)
    pub fn prob_with_confidence(
        current_price: f64,
        strike_price: f64,
        time_remaining_sec: f64,
        vol_per_sec: f64,
        confidence_level: f64,
    ) -> (f64, f64, f64) {
        let prob = Self::prob_up(
            current_price,
            strike_price,
            time_remaining_sec,
            vol_per_sec,
            0.0,
        );

        let std = vol_per_sec * time_remaining_sec.sqrt();
        let distance = current_price - strike_price;

        // Use delta method for confidence interval
        // For a binary option, this is approximate
        let z = distance / std.max(1e-6);
        let pdf = norm_pdf(z); // PDF at current z

        // Width of confidence interval
        let width = confidence_level * pdf * std;

        let lower = (prob - width).max(0.0);
        let upper = (prob + width).min(1.0);

        (prob, lower, upper)
    }

    /// Calculate edge (difference between fair and market price)
    ///
    /// # Arguments
    /// * `fair_prob` - Fair value probability from model
    /// * `market_yes_ask` - Market YES ask price
    /// * `market_no_ask` - Market NO ask price
    ///
    /// # Returns
    /// (edge_yes, edge_no) - positive means fair > market (good to buy)
    pub fn calculate_edge(fair_prob: f64, market_yes_ask: f64, market_no_ask: f64) -> (f64, f64) {
        let edge_yes = fair_prob - market_yes_ask;
        let edge_no = (1.0 - fair_prob) - market_no_ask;
        (edge_yes, edge_no)
    }

    /// Calculate time-decay adjusted probability
    ///
    /// As time approaches expiration, probabilities should converge toward
    /// their intrinsic values based on current price vs strike.
    ///
    /// # Arguments
    /// * `current_prob` - Current probability estimate
    /// * `total_time_sec` - Total duration of the market
    /// * `elapsed_time_sec` - Time already elapsed
    /// * `price_above_strike` - Whether current price is above strike
    ///
    /// # Returns
    /// Time-adjusted probability
    pub fn time_decay_adjustment(
        current_prob: f64,
        total_time_sec: f64,
        elapsed_time_sec: f64,
        price_above_strike: bool,
    ) -> f64 {
        let progress = (elapsed_time_sec / total_time_sec).min(1.0);

        if price_above_strike {
            // Drift toward 1.0
            current_prob + (1.0 - current_prob) * progress * 0.5
        } else {
            // Drift toward 0.0
            current_prob * (1.0 - progress * 0.5)
        }
    }

    /// Calculate the probability that the parent resolves YES given
    /// that some children have already resolved
    ///
    /// This is a key function for temporal arbitrage. After observing
    /// some child resolutions, we can update our belief about the parent.
    ///
    /// # Arguments
    /// * `num_yes_children` - Number of children that resolved YES
    /// * `num_no_children` - Number of children that resolved NO
    /// * `live_price` - Current live BTC price
    /// * `parent_strike` - Parent's strike price
    /// * `time_per_child` - Duration of each child interval
    /// * `children_remaining` - Number of children yet to resolve
    /// * `vol_per_sec` - Volatility per second
    ///
    /// # Returns
    /// Updated parent probability
    pub fn parent_prob_given_children(
        num_yes_children: usize,
        num_no_children: usize,
        live_price: f64,
        parent_strike: f64,
        time_per_child: f64,
        children_remaining: usize,
        vol_per_sec: f64,
    ) -> f64 {
        let total_resolved = num_yes_children + num_no_children;

        if total_resolved == 0 {
            return Self::prob_up(
                live_price,
                parent_strike,
                time_per_child * children_remaining as f64,
                vol_per_sec,
                0.0,
            );
        }

        // Base probability from current price
        let remaining_time = time_per_child * children_remaining as f64;
        let base_prob = Self::prob_up(live_price, parent_strike, remaining_time, vol_per_sec, 0.0);

        // Empirical signal from resolved children
        let empirical_signal = num_yes_children as f64 / total_resolved as f64;

        // Weight by confidence in the signal
        let confidence =
            (total_resolved as f64 / (total_resolved + children_remaining) as f64).min(0.5);

        // Blend base model with empirical signal
        base_prob * (1.0 - confidence) + empirical_signal * confidence
    }
}

/// Standard normal CDF (Cumulative Distribution Function)
///
/// Uses the Abramowitz and Stegun approximation (7.1.26)
/// with maximum error < 7.5e-8
///
/// # Formula
/// ```text
/// Φ(z) = 1 - φ(z) * (a1*t + a2*t^2 + a3*t^3 + a4*t^4 + a5*t^5)
///
/// where:
/// - t = 1 / (1 + p*|z|)
/// - p = 0.2316419
/// - a1 = 0.319381530, a2 = -0.356563782, a3 = 1.781477937
/// - a4 = -1.821255978, a5 = 1.330274429
/// ```
fn norm_cdf(z: f64) -> f64 {
    const A1: f64 = 0.254829592;
    const A2: f64 = -0.284496736;
    const A3: f64 = 1.421413741;
    const A4: f64 = -1.453152027;
    const A5: f64 = 1.061405429;
    const P: f64 = 0.3275911;

    let abs_z = z.abs();

    // Handle extreme values
    if abs_z > 37.0 {
        return if z > 0.0 { 1.0 } else { 0.0 };
    }

    let t = 1.0 / (1.0 + P * abs_z);
    let y =
        1.0 - (((((A5 * t + A4) * t) + A3) * t + A2) * t + A1) * t * (-0.5 * abs_z * abs_z).exp();

    if z > 0.0 {
        y
    } else {
        1.0 - y
    }
}

/// Standard normal PDF (Probability Density Function)
///
/// # Formula
/// ```text
/// φ(z) = (1/√(2π)) * e^(-z²/2)
/// ```
fn norm_pdf(z: f64) -> f64 {
    const SQRT_2PI: f64 = 2.5066282746310002; // sqrt(2π)
    (-0.5 * z * z).exp() / SQRT_2PI
}

/// Inverse normal CDF (quantile function)
///
/// Uses the Beasley-Springer-Moro approximation
pub fn norm_inv(p: f64) -> f64 {
    debug_assert!(p > 0.0 && p < 1.0, "p must be in (0,1), got {}", p);

    // Coefficients for rational approximation
    const A: [f64; 4] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383577518672690e+02,
    ];
    const B: [f64; 4] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
    ];
    const C: [f64; 4] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];

    let p = p.clamp(1e-10, 1.0 - 1e-10);
    let q = p - 0.5;

    if q.abs() <= 0.425 {
        // Central region
        let r = 0.180625 - q * q;
        let num = ((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r) * q;
        let den = ((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r) + 1.0;
        num / den
    } else {
        let r = if q < 0.0 { p } else { 1.0 - p };

        let r = (-r.ln()).sqrt();
        let num = ((((C[0] * r + C[1]) * r + C[2]) * r + C[3]) * r + C[3]) * r;
        let den = (((D[0] * r + D[1]) * r + D[2]) * r + D[3]) * r + 1.0;

        let result = num / den;

        if q < 0.0 {
            -result
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_norm_cdf_properties() {
        // Φ(0) = 0.5
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-10);

        // Φ(-∞) → 0, Φ(∞) → 1
        assert!((norm_cdf(-10.0) - 0.0).abs() < 0.001);
        assert!((norm_cdf(10.0) - 1.0).abs() < 0.001);

        // Symmetry: Φ(-x) = 1 - Φ(x)
        for z in [-5.0, -2.0, -1.0, 0.0, 1.0, 2.0, 5.0] {
            assert!((norm_cdf(-z) - (1.0 - norm_cdf(z))).abs() < 1e-10);
        }
    }

    #[test]
    fn test_norm_pdf_properties() {
        // φ(0) = 1/√(2π) ≈ 0.3989
        assert!((norm_pdf(0.0) - 0.3989422804014327).abs() < 1e-10);

        // Symmetry: φ(-x) = φ(x)
        for z in [0.0, 1.0, 2.0, 5.0] {
            assert!((norm_pdf(-z) - norm_pdf(z)).abs() < 1e-10);
        }

        // φ decreases as |z| increases
        assert!(norm_pdf(0.0) > norm_pdf(1.0));
        assert!(norm_pdf(1.0) > norm_pdf(2.0));
    }

    #[test]
    fn test_prob_up_at_money() {
        let current_price = 71000.0;
        let strike_price = 71000.0;
        let time_remaining = 300.0;
        let vol_per_sec = 250.0 / 300.0;

        let prob = ProbabilityEngine::prob_up(
            current_price,
            strike_price,
            time_remaining,
            vol_per_sec,
            0.0,
        );

        // At-the-money should be near 0.5
        assert!((prob - 0.5).abs() < 0.1);
    }

    #[test]
    fn test_prob_up_deep_in_money() {
        let current_price = 72000.0;
        let strike_price = 71000.0;
        let time_remaining = 300.0;
        let vol_per_sec = 250.0 / 300.0;

        let prob = ProbabilityEngine::prob_up(
            current_price,
            strike_price,
            time_remaining,
            vol_per_sec,
            0.0,
        );

        // Deep in-the-money should be near 1.0
        assert!(prob > 0.9);
    }

    #[test]
    fn test_prob_up_deep_out_of_money() {
        let current_price = 70000.0;
        let strike_price = 71000.0;
        let time_remaining = 300.0;
        let vol_per_sec = 250.0 / 300.0;

        let prob = ProbabilityEngine::prob_up(
            current_price,
            strike_price,
            time_remaining,
            vol_per_sec,
            0.0,
        );

        // Deep out-of-the-money should be near 0.0
        assert!(prob < 0.1);
    }

    #[test]
    fn test_chained_conditional_no_legs() {
        let observed_legs: &[(f64, bool)] = &[];
        let original_strike = 71000.0;
        let live_btc_price = 71000.0;
        let remaining_time = 600.0;
        let vol_per_sec = 250.0 / 300.0;

        let prob = ProbabilityEngine::chained_conditional_after_observed_legs(
            observed_legs,
            original_strike,
            live_btc_price,
            remaining_time,
            vol_per_sec,
            0.0,
        );

        // Should fall back to base probability
        assert!((prob - 0.5).abs() < 0.1);
    }

    #[test]
    fn test_chained_conditional_above_strike() {
        let observed_legs = &[(71050.0, true), (71030.0, false)];
        let original_strike = 71000.0;
        let live_btc_price = 71050.0;
        let remaining_time = 300.0;
        let vol_per_sec = 250.0 / 300.0;

        let prob = ProbabilityEngine::chained_conditional_after_observed_legs(
            observed_legs,
            original_strike,
            live_btc_price,
            remaining_time,
            vol_per_sec,
            0.0,
        );

        // Above strike should give high probability
        assert!(prob > 0.9);
    }

    #[test]
    fn test_implied_parent_prob_after_k_legs() {
        let k_yes = 2;
        let k_total = 3;
        let live_price = 71000.0;
        let original_strike = 71000.0;
        let remaining_time = 600.0;
        let vol_per_sec = 250.0 / 300.0;

        let prob = ProbabilityEngine::implied_parent_prob_after_k_legs(
            k_yes,
            k_total,
            live_price,
            original_strike,
            remaining_time,
            vol_per_sec,
        );

        // Should be between 0.5 and 0.8
        assert!(prob > 0.4 && prob < 0.9);
    }

    #[test]
    fn test_calculate_edge() {
        let fair_prob = 0.60;
        let market_yes_ask = 0.50;
        let market_no_ask = 0.48;

        let (edge_yes, edge_no) =
            ProbabilityEngine::calculate_edge(fair_prob, market_yes_ask, market_no_ask);

        // Yes edge should be positive (fair > market)
        assert!((edge_yes - 0.10).abs() < 1e-10);

        // No edge should be negative (fair NO = 0.40 < market NO = 0.48)
        assert!(edge_no < 0.0);
    }

    #[test]
    fn test_time_decay_adjustment() {
        let current_prob = 0.60;
        let total_time = 300.0;
        let elapsed_time = 150.0;

        // Above strike case
        let adj_above =
            ProbabilityEngine::time_decay_adjustment(current_prob, total_time, elapsed_time, true);
        assert!(adj_above > current_prob);

        // Below strike case
        let adj_below =
            ProbabilityEngine::time_decay_adjustment(current_prob, total_time, elapsed_time, false);
        assert!(adj_below < current_prob);
    }

    #[test]
    fn test_parent_prob_given_children() {
        let num_yes_children = 2;
        let num_no_children = 1;
        let live_price = 71050.0;
        let parent_strike = 71000.0;
        let time_per_child = 300.0;
        let children_remaining = 1;
        let vol_per_sec = 250.0 / 300.0;

        let prob = ProbabilityEngine::parent_prob_given_children(
            num_yes_children,
            num_no_children,
            live_price,
            parent_strike,
            time_per_child,
            children_remaining,
            vol_per_sec,
        );

        // Should be > 0.5 since 2/3 children resolved YES and price above strike
        assert!(prob > 0.5);
    }

    #[test]
    fn test_norm_inv() {
        // Round-trip test: inv(cdf(z)) ≈ z
        for z in [-2.0, -1.0, 0.0, 1.0, 2.0] {
            let p = norm_cdf(z);
            let z_back = norm_inv(p);
            assert!((z - z_back).abs() < 1e-6);
        }
    }

    #[test]
    fn test_violation_type_display() {
        assert_eq!(
            format!("{}", ViolationType::ChainedConditional),
            "ChainedConditional"
        );
        assert_eq!(
            format!("{}", ViolationType::LatePhaseAnomaly),
            "LatePhaseAnomaly"
        );
        assert_eq!(
            format!("{}", ViolationType::PriceConsistency),
            "PriceConsistency"
        );
    }
}
