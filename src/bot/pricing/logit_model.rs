//! Logit Jump-Diffusion with Risk-Neutral Drift
//!
//! Based on arxiv:2510.15205 "Toward Black–Scholes for Prediction Markets"
//!
//! This module implements a pricing model that:
//! 1. Works in logit space (unbounded log-odds) to respect probability bounds
//! 2. Uses risk-neutral drift derived from the martingale condition
//! 3. Explicitly models jumps via a compound Poisson process
//!
//! The core insight from the paper is that the drift μ is NOT a free parameter -
//! it's determined by requiring p_t = S(x_t) to be a martingale under the
//! risk-neutral measure.

use serde::Serialize;

/// Sigmoid function: p = S(x) = 1/(1+e^(-x))
///
/// Transforms logit (unbounded) to probability (bounded in (0,1))
pub fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// First derivative of sigmoid: S'(x) = p(1-p)
///
/// This is maximized at x=0 (p=0.5) where S'(0) = 0.25
pub fn sigmoid_prime(x: f64) -> f64 {
    let p = sigmoid(x);
    p * (1.0 - p)
}

/// Second derivative of sigmoid: S''(x) = p(1-p)(1-2p)
///
/// Note: S''(0) = 0, which simplifies the risk-neutral drift at p=0.5
pub fn sigmoid_double_prime(x: f64) -> f64 {
    let p = sigmoid(x);
    let s_prime = p * (1.0 - p);
    s_prime * (1.0 - 2.0 * p)
}

/// Transform probability to logit space
///
/// logit(p) = log(p / (1-p))
///
/// # Panics
/// Panics in debug mode if p is not in (0,1)
pub fn prob_to_logit(p: f64) -> f64 {
    debug_assert!(p > 0.0 && p < 1.0, "prob must be in (0,1), got {}", p);
    (p / (1.0 - p)).ln()
}

/// Transform logit to probability space
///
/// Uses sigmoid for numerical stability
pub fn logit_to_prob(x: f64) -> f64 {
    sigmoid(x)
}

/// Risk-neutral drift calculation (Equation 3 from paper)
///
/// This ensures p_t = S(x_t) is a martingale under the risk-neutral measure.
/// The drift is NOT a free parameter - it's determined by the martingale condition.
///
/// # Formula
/// ```text
/// μ(t,x) = -[½ S''(x) σ_b²(t,x) + ∫(S(x+z) - S(x) - S'(x)χ(z)) ν_t(dz)] / S'(x)
/// ```
///
/// At x=0 (p=0.5), S''(0)=0, so the drift is determined primarily by the jump compensator.
///
/// # Arguments
/// * `x` - Current logit value
/// * `vol` - Belief volatility σ_b (in logit space, annualized)
/// * `jump_compensator` - Jump integral term from the measure change
///
/// # Returns
/// The risk-neutral drift rate (in logit units per year)
pub fn risk_neutral_drift(
    x: f64,
    vol: f64,
    jump_compensator: f64,
) -> f64 {
    let s_prime = sigmoid_prime(x);
    let s_double_prime = sigmoid_double_prime(x);

    // Guard against division by zero at extreme probabilities
    let s_prime = s_prime.max(1e-10);

    // μ = -(½ S''(x) σ² + jump_compensator) / S'(x)
    let diffusion_drift = -0.5 * s_double_prime * vol * vol;
    (diffusion_drift - jump_compensator) / s_prime
}

/// Jump compensator: ∫(S(x+z) - S(x) - S'(x)χ(z)) ν(dz)
///
/// This term compensates for the expected impact of jumps to maintain the martingale property.
/// For small jumps, we use a Taylor expansion approximation.
///
/// # Arguments
/// * `lambda` - Jump intensity (arrivals per year)
/// * `jump_second_moment` - E[z²] where z is the jump size in logit space
///
/// # Returns
/// The compensator term for the risk-neutral drift
pub fn jump_compensator(lambda: f64, jump_second_moment: f64) -> f64 {
    // For small jumps, use Taylor expansion:
    // ≈ ½ S''(x) * E[z²] * λ
    0.5 * lambda * jump_second_moment
}

/// Market observation in probability space with metadata
#[derive(Debug, Clone, Serialize)]
pub struct LogitObservation {
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
    /// Observed market probability
    pub prob: f64,
    /// Pre-computed logit value (for efficiency)
    pub logit: f64,
    /// Bid-ask spread (for measurement noise estimation)
    pub spread: f64,
    /// Volume/liquidity metric (for quality weighting)
    pub volume: f64,
}

impl LogitObservation {
    /// Create a new observation from raw market data
    pub fn from_market(timestamp: i64, yes_bid: f64, yes_ask: f64, volume: f64) -> Self {
        let prob = (yes_bid + yes_ask) / 2.0;
        let spread = yes_ask - yes_bid;
        Self {
            timestamp,
            prob: prob.clamp(0.001, 0.999),
            logit: prob_to_logit(prob.clamp(0.001, 0.999)),
            spread,
            volume,
        }
    }
}

/// Calibrated logit jump-diffusion model
///
/// This model maintains a filtered state estimate of the "true" logit value,
/// accounting for microstructure noise and providing fair value predictions
/// via the risk-neutral measure.
#[derive(Debug, Clone)]
pub struct LogitJumpDiffusion {
    /// Current filtered logit estimate
    current_logit: f64,
    /// Current probability estimate (sigmoid of logit)
    current_prob: f64,
    /// Current belief volatility estimate
    current_vol: f64,

    /// Jump parameters (calibrated from data)
    jump_intensity: f64,     // λ: Poisson arrival rate
    jump_second_moment: f64, // E[z²]

    /// Configuration bounds
    min_vol: f64,
    max_vol: f64,
}

impl LogitJumpDiffusion {
    /// Create a new model with default calibration
    pub fn new() -> Self {
        Self {
            current_logit: 0.0,
            current_prob: 0.5,
            current_vol: 0.8,  // ~80% annualized at p=0.5
            jump_intensity: 0.1,  // ~0.1 jumps/day ≈ 36.5 jumps/year
            jump_second_moment: 0.3,  // Average jump variance
            min_vol: 0.2,
            max_vol: 2.0,
        }
    }

    /// Create with custom parameters
    pub fn with_params(
        initial_prob: f64,
        base_vol: f64,
        jump_intensity: f64,
        jump_second_moment: f64,
    ) -> Self {
        let logit = prob_to_logit(initial_prob.clamp(0.001, 0.999));
        Self {
            current_logit: logit,
            current_prob: initial_prob.clamp(0.001, 0.999),
            current_vol: base_vol,
            jump_intensity,
            jump_second_moment,
            min_vol: 0.1,
            max_vol: 3.0,
        }
    }

    /// Update with new observation (returns filtered estimate)
    ///
    /// This applies an exponential filter to the observed logit,
    /// with adaptive gain based on spread and volume.
    pub fn update(&mut self, obs: &LogitObservation) -> FilteredState {
        // Transform observation to logit space (already done, but validate)
        let observed_logit = prob_to_logit(obs.prob.clamp(0.001, 0.999));

        // Apply exponential smoothing with adaptive gain
        let alpha = self.adaptive_gain(obs.spread, obs.volume);
        self.current_logit = alpha * observed_logit + (1.0 - alpha) * self.current_logit;
        self.current_prob = sigmoid(self.current_logit);

        // Update volatility estimate (U-shaped surface)
        self.current_vol = self.belief_volatility(self.current_prob);

        FilteredState {
            logit: self.current_logit,
            prob: self.current_prob,
            vol: self.current_vol,
        }
    }

    /// Belief volatility: U-shaped in probability space
    ///
    /// σ_b(p) = σ_min + (σ_base - σ_min) * 4p(1-p)
    ///
    /// This captures the empirical observation that belief volatility
    /// is highest at p=0.5 (maximum uncertainty) and lowest near boundaries.
    fn belief_volatility(&self, p: f64) -> f64 {
        let u_shape = 4.0 * p * (1.0 - p);
        let base_vol = 0.8;
        let min_vol = 0.2;
        (min_vol + (base_vol - min_vol) * u_shape).clamp(self.min_vol, self.max_vol)
    }

    /// Calculate fair probability for future time horizon
    ///
    /// This returns the risk-neutral expectation of p_T given current state,
    /// with confidence intervals derived from the logit normal distribution.
    ///
    /// # Arguments
    /// * `time_horizon_s` - Time horizon in seconds
    ///
    /// # Returns
    /// Fair probability with confidence bounds and diagnostic information
    pub fn fair_prob(&self, time_horizon_s: i64) -> FairProbability {
        // Convert to years (standard for volatility quoting)
        let dt = (time_horizon_s as f64).max(0.0) / (365.25 * 24.0 * 3600.0);

        // Calculate risk-neutral drift (martingale-preserving)
        let jc = jump_compensator(self.jump_intensity, self.jump_second_moment);
        let drift = risk_neutral_drift(self.current_logit, self.current_vol, jc);

        // Expected logit at horizon (under risk-neutral measure)
        // E[x_T] = x_0 + μ * T
        let expected_logit = self.current_logit + drift * dt;

        // Logit standard deviation: σ * sqrt(T)
        let logit_std = self.current_vol * dt.sqrt();

        // Transform to probability space using sigmoid
        let expected_prob = sigmoid(expected_logit);

        // Confidence interval using delta method
        // For small T: p_lower ≈ S(E[x] - z*σ), p_upper ≈ S(E[x] + z*σ)
        let z = 1.96;  // 95% CI
        let prob_lower = sigmoid(expected_logit - z * logit_std);
        let prob_upper = sigmoid(expected_logit + z * logit_std);

        FairProbability {
            expected: expected_prob,
            lower: prob_lower,
            upper: prob_upper,
            logit_mean: expected_logit,
            logit_std,
            drift,
        }
    }

    /// Get current filtered state
    pub fn state(&self) -> FilteredState {
        FilteredState {
            logit: self.current_logit,
            prob: self.current_prob,
            vol: self.current_vol,
        }
    }

    /// Adaptive Kalman gain based on spread and volume
    ///
    /// High spread → low trust → low gain
    /// High volume → high trust → high gain
    fn adaptive_gain(&self, spread: f64, volume: f64) -> f64 {
        let base_gain = 0.1;
        let spread_penalty = (spread.min(0.1) / 0.1).min(1.0);
        let volume_boost = (volume / 1000.0).min(1.0);
        (base_gain * (1.0 - spread_penalty * 0.5) + volume_boost * 0.05)
            .clamp(0.01, 0.5)
    }

    /// Calculate edge vs market price
    ///
    /// Positive edge means fair value > market (buy YES)
    /// Negative edge means fair value < market (buy NO)
    pub fn edge(&self, fair_prob: f64, market_ask: f64) -> f64 {
        fair_prob - market_ask
    }

    /// Update jump parameters from external calibration
    pub fn calibrate_jumps(&mut self, intensity: f64, second_moment: f64) {
        self.jump_intensity = intensity.clamp(0.001, 100.0);
        self.jump_second_moment = second_moment.clamp(0.01, 10.0);
    }
}

/// Filtered state estimate from the model
#[derive(Debug, Clone, Serialize)]
pub struct FilteredState {
    /// Filtered logit estimate
    pub logit: f64,
    /// Corresponding probability (sigmoid of logit)
    pub prob: f64,
    /// Current belief volatility estimate
    pub vol: f64,
}

/// Fair value prediction for a future time
#[derive(Debug, Clone, Serialize)]
pub struct FairProbability {
    /// Expected probability under risk-neutral measure
    pub expected: f64,
    /// Lower bound of 95% confidence interval
    pub lower: f64,
    /// Upper bound of 95% confidence interval
    pub upper: f64,
    /// Expected logit value (diagnostic)
    pub logit_mean: f64,
    /// Logit standard deviation (diagnostic)
    pub logit_std: f64,
    /// Risk-neutral drift rate (diagnostic)
    pub drift: f64,
}

impl Default for LogitJumpDiffusion {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigmoid_properties() {
        // S(0) = 0.5 exactly
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-10);

        // S(-∞) → 0, S(∞) → 1
        assert!((sigmoid(-10.0) - 0.0).abs() < 0.001);
        assert!((sigmoid(10.0) - 1.0).abs() < 0.001);

        // Symmetry: S(-x) = 1 - S(x)
        for x in [-5.0, -2.0, -1.0, 0.0, 1.0, 2.0, 5.0] {
            assert!((sigmoid(-x) - (1.0 - sigmoid(x))).abs() < 1e-10);
        }
    }

    #[test]
    fn test_sigmoid_prime_maximum_at_center() {
        // S'(x) = p(1-p) is maximized at p=0.5 (x=0)
        let s_prime_0 = sigmoid_prime(0.0);
        assert!((s_prime_0 - 0.25).abs() < 1e-10);

        // Should be smaller away from center
        assert!(sigmoid_prime(-2.0) < s_prime_0);
        assert!(sigmoid_prime(2.0) < s_prime_0);
    }

    #[test]
    fn test_sigmoid_double_prime_zero_at_center() {
        // S''(0) = 0 (critical property for drift simplification)
        assert!(sigmoid_double_prime(0.0).abs() < 1e-10);

        // Positive for x < 0 (p < 0.5)
        assert!(sigmoid_double_prime(-1.0) > 0.0);

        // Negative for x > 0 (p > 0.5)
        assert!(sigmoid_double_prime(1.0) < 0.0);
    }

    #[test]
    fn test_logit_roundtrip() {
        // Test various probabilities
        for p in [0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99] {
            let x = prob_to_logit(p);
            let p_back = logit_to_prob(x);
            assert!((p - p_back).abs() < 1e-10);
        }
    }

    #[test]
    fn test_logit_zero_at_half() {
        // logit(0.5) = 0
        assert!(prob_to_logit(0.5).abs() < 1e-10);
        assert!((logit_to_prob(0.0) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_risk_neutral_drift_at_center() {
        // At x=0 (p=0.5), S''(0)=0, so drift ≈ -jump_compensator / S'(0)
        let drift = risk_neutral_drift(0.0, 0.8, 0.0);
        assert!(drift.abs() < 1e-10);

        // With jumps, drift is determined by compensator
        let jc = jump_compensator(10.0, 0.3);
        let drift_with_jumps = risk_neutral_drift(0.0, 0.8, jc);
        // At x=0: drift ≈ -jc / S'(0) = -jc / 0.25 = -4*jc
        let expected = -4.0 * jc;
        assert!((drift_with_jumps - expected).abs() < 0.01);
    }

    #[test]
    fn test_risk_neutral_drift_bounds() {
        // Drift should be well-behaved across probability range
        for p in [0.1, 0.25, 0.5, 0.75, 0.9] {
            let x = prob_to_logit(p);
            let drift = risk_neutral_drift(x, 0.8, 0.01);
            assert!(drift.is_finite());
            assert!(drift.abs() < 100.0); // Sanity check
        }
    }

    #[test]
    fn test_jump_compensator_positive() {
        // Jump compensator should always be positive
        let jc = jump_compensator(10.0, 0.3);
        assert!(jc > 0.0);

        // Scales with intensity and second moment
        let jc2 = jump_compensator(20.0, 0.3);
        assert!(jc2 > jc);

        let jc3 = jump_compensator(10.0, 0.6);
        assert!(jc3 > jc);
    }

    #[test]
    fn test_martingale_property_zero_horizon() {
        // At T=0, fair value should equal current value
        let model = LogitJumpDiffusion::new();
        let fair = model.fair_prob(0);

        // Expected probability should be close to initial (0.5)
        assert!((fair.expected - 0.5).abs() < 0.01);

        // Confidence interval should collapse to a point
        assert!((fair.lower - fair.upper).abs() < 0.01);
    }

    #[test]
    fn test_belief_volatility_u_shape() {
        let model = LogitJumpDiffusion::new();

        // Highest at p=0.5
        let vol_mid = model.belief_volatility(0.5);

        // Lower at extremes
        let vol_low = model.belief_volatility(0.1);
        let vol_high = model.belief_volatility(0.9);

        assert!(vol_mid > vol_low);
        assert!(vol_mid > vol_high);

        // Symmetric
        assert!((vol_low - vol_high).abs() < 0.01);
    }

    #[test]
    fn test_probability_bounds() {
        // Even with extreme logit values, prob stays in (0,1)
        assert!((sigmoid(-100.0) - 0.0).abs() < 1e-40);
        assert!((sigmoid(100.0) - 1.0).abs() < 1e-40);

        // Test fair probability bounds
        let model = LogitJumpDiffusion::with_params(0.5, 2.0, 0.0, 0.0);
        let fair = model.fair_prob(3600); // 1 hour

        assert!(fair.expected > 0.0 && fair.expected < 1.0);
        assert!(fair.lower > 0.0 && fair.lower < 1.0);
        assert!(fair.upper > 0.0 && fair.upper < 1.0);
    }

    #[test]
    fn test_update_converges_to_observed() {
        let mut model = LogitJumpDiffusion::new();

        // Start at 0.5
        assert!((model.current_prob - 0.5).abs() < 0.01);

        // Feed observations at 0.7
        for _ in 0..100 {
            let obs = LogitObservation {
                timestamp: 0,
                prob: 0.7,
                logit: prob_to_logit(0.7),
                spread: 0.01,
                volume: 1000.0,
            };
            model.update(&obs);
        }

        // Should converge close to 0.7
        assert!((model.current_prob - 0.7).abs() < 0.05);
    }

    #[test]
    fn test_observation_from_market() {
        let obs = LogitObservation::from_market(1000, 0.49, 0.51, 5000.0);

        assert_eq!(obs.timestamp, 1000);
        assert_eq!(obs.prob, 0.5);
        assert_eq!(obs.spread, 0.02);
        assert_eq!(obs.volume, 5000.0);
        // logit(0.5) = 0
        assert!(obs.logit.abs() < 1e-10);
    }

    #[test]
    fn test_edge_calculation() {
        let model = LogitJumpDiffusion::new();

        // Fair value higher than market → positive edge (buy YES)
        let edge = model.edge(0.55, 0.50);
        assert!((edge - 0.05).abs() < 1e-10);

        // Fair value lower than market → negative edge (buy NO)
        let edge = model.edge(0.45, 0.50);
        assert!((edge - (-0.05)).abs() < 1e-10);
    }

    #[test]
    fn test_with_params() {
        let model = LogitJumpDiffusion::with_params(0.7, 1.0, 5.0, 0.5);

        assert!((model.current_prob - 0.7).abs() < 0.01);
        assert_eq!(model.current_vol, 1.0);
        assert_eq!(model.jump_intensity, 5.0);
        assert_eq!(model.jump_second_moment, 0.5);
    }

    #[test]
    fn test_calibrate_jumps() {
        let mut model = LogitJumpDiffusion::new();

        model.calibrate_jumps(5.0, 0.8);

        assert_eq!(model.jump_intensity, 5.0);
        assert_eq!(model.jump_second_moment, 0.8);
    }

    #[test]
    fn test_adaptive_gain() {
        let model = LogitJumpDiffusion::new();

        // High spread → lower gain
        let gain_wide = model.adaptive_gain(0.05, 1000.0);
        let gain_tight = model.adaptive_gain(0.005, 1000.0);
        assert!(gain_tight > gain_wide);

        // High volume → higher gain
        let gain_low_vol = model.adaptive_gain(0.01, 100.0);
        let gain_high_vol = model.adaptive_gain(0.01, 10000.0);
        assert!(gain_high_vol > gain_low_vol);
    }
}
