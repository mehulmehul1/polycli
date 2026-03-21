//! Heteroskedastic Kalman Filter for Logit Space
//!
//! Filters microstructure noise from observed probabilities.
//! Based on Section 5.1 of arxiv:2510.15205
//!
//! The Kalman filter maintains:
//! - State estimate x (logit value)
//! - State covariance P (uncertainty in estimate)
//! - Process noise Q (volatility-driven state evolution)
//! - Measurement noise R (bid-ask spread proxy)

/// Heteroskedastic Kalman Filter for logit probabilities
///
/// This filter separates the "true" logit value from microstructure noise
/// by modeling the observation process with variance proportional to spread².
#[derive(Debug, Clone)]
pub struct KalmanFilter {
    /// State estimate (logit value)
    x: f64,
    /// State covariance (uncertainty)
    p: f64,
    /// Process noise (volatility squared)
    q: f64,
    /// Measurement noise (from spread)
    r: f64,
}

impl KalmanFilter {
    /// Create a new filter with initial logit estimate
    pub fn new(initial_logit: f64) -> Self {
        Self {
            x: initial_logit,
            p: 1.0,  // Initial uncertainty
            q: 0.1,  // Initial process noise
            r: 0.05, // Initial measurement noise
        }
    }

    /// Create with default initial state (logit = 0, prob = 0.5)
    pub fn default() -> Self {
        Self::new(0.0)
    }

    /// Predict step: propagate state forward
    ///
    /// Under the risk-neutral dynamics:
    /// x_pred = x + μ*dt
    /// P_pred = P + Q*dt
    ///
    /// # Arguments
    /// * `dt` - Time step in years
    /// * `drift` - Risk-neutral drift rate μ
    /// * `vol` - Belief volatility σ_b
    pub fn predict(&mut self, dt: f64, drift: f64, vol: f64) {
        // x_pred = x + drift * dt
        self.x += drift * dt;

        // P_pred = P + Q * dt
        self.p += self.q * dt;

        // Process noise scales with volatility
        self.q = vol * vol;

        // Ensure numerical stability
        self.p = self.p.clamp(1e-10, 100.0);
    }

    /// Update step: incorporate measurement
    ///
    /// The Kalman gain determines how much to trust the new measurement:
    /// - High spread → high R → low gain
    /// - Low P (high confidence) → low gain
    ///
    /// # Arguments
    /// * `measurement_logit` - Observed logit value
    /// * `spread` - Bid-ask spread (for measurement noise estimation)
    pub fn update(&mut self, measurement_logit: f64, spread: f64) {
        // Measurement noise increases with spread
        self.set_measurement_noise(spread);

        // Kalman gain: K = P / (P + R)
        let k = self.p / (self.p + self.r);

        // State update: x = x + K * (z - x)
        let innovation = measurement_logit - self.x;
        self.x += k * innovation;

        // Covariance update: P = P * (1 - K)
        self.p *= 1.0 - k;

        // Ensure numerical stability
        self.p = self.p.clamp(1e-10, 10.0);
    }

    /// Set measurement noise based on spread
    ///
    /// σ_η² ∝ spread²
    /// The spread represents microstructure noise - wider spread means
    /// less confidence in the observed price.
    pub fn set_measurement_noise(&mut self, spread: f64) {
        // Convert price spread to logit-space noise
        // For small spread: logit_spread ≈ spread / (4 * p * (1-p))
        // We use a conservative approximation
        self.r = (spread * spread / 4.0).clamp(1e-6, 0.5);
    }

    /// Get current state estimate
    pub fn state(&self) -> f64 {
        self.x
    }

    /// Get probability (sigmoid of state)
    pub fn probability(&self) -> f64 {
        crate::bot::pricing::logit_model::sigmoid(self.x)
    }

    /// Get uncertainty (standard deviation)
    pub fn uncertainty(&self) -> f64 {
        self.p.sqrt()
    }

    /// Get current process noise
    pub fn process_noise(&self) -> f64 {
        self.q
    }

    /// Get current measurement noise
    pub fn measurement_noise(&self) -> f64 {
        self.r
    }

    /// Reset to initial state
    pub fn reset(&mut self, initial_logit: f64) {
        self.x = initial_logit;
        self.p = 1.0;
        self.q = 0.1;
        self.r = 0.05;
    }
}

impl Default for KalmanFilter {
    fn default() -> Self {
        Self::new(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kalman_initial_state() {
        let kf = KalmanFilter::new(0.0);
        assert_eq!(kf.state(), 0.0);
        assert_eq!(kf.probability(), 0.5);
    }

    #[test]
    fn test_predict_step() {
        let mut kf = KalmanFilter::new(0.0);

        // Predict forward with drift
        kf.predict(0.01, 0.5, 0.8);

        // x should increase due to drift
        assert!(kf.state() > 0.0);
        assert!(kf.state() < 1.0); // Should be small
    }

    #[test]
    fn test_update_toward_measurement() {
        let mut kf = KalmanFilter::new(0.0);

        // Update with measurement above current state
        kf.update(0.5, 0.01);

        // State should move toward measurement
        assert!(kf.state() > 0.0);
        assert!(kf.state() < 0.5); // But not all the way

        // Uncertainty should decrease
        assert!(kf.uncertainty() < 1.0);
    }

    #[test]
    fn test_wide_spread_reduces_gain() {
        let mut kf1 = KalmanFilter::new(0.0);
        let mut kf2 = KalmanFilter::new(0.0);

        kf1.update(0.5, 0.001); // Tight spread
        kf2.update(0.5, 0.05); // Wide spread

        // Tight spread should trust measurement more
        assert!(kf1.state() > kf2.state());
    }

    #[test]
    fn test_kalman_convergence() {
        let mut kf = KalmanFilter::new(0.0);
        let true_value = 0.5;

        for i in 0..100 {
            let noisy = true_value + (rand::random::<f64>() - 0.5) * 0.1;
            kf.predict(0.01, 0.0, 0.1);
            kf.update(noisy, 0.02);
        }

        // Should converge close to true value
        assert!((kf.state() - 0.5).abs() < 0.1);
    }

    #[test]
    fn test_uncertainty_decreases_with_observations() {
        let mut kf = KalmanFilter::new(0.0);
        let initial_uncertainty = kf.uncertainty();

        kf.update(0.1, 0.01);

        assert!(kf.uncertainty() < initial_uncertainty);
    }

    #[test]
    fn test_measurement_noise_from_spread() {
        let mut kf = KalmanFilter::new(0.0);

        kf.set_measurement_noise(0.01);
        let r_small = kf.measurement_noise();

        kf.set_measurement_noise(0.05);
        let r_large = kf.measurement_noise();

        assert!(r_large > r_small);
    }

    #[test]
    fn test_reset() {
        let mut kf = KalmanFilter::new(0.0);

        kf.predict(1.0, 1.0, 1.0);
        kf.update(1.0, 0.01);

        kf.reset(1.0);

        assert_eq!(kf.state(), 1.0);
    }

    #[test]
    fn test_probability_bounds() {
        let kf = KalmanFilter::new(-100.0);
        assert!(kf.probability() < 0.01);

        let kf = KalmanFilter::new(100.0);
        assert!(kf.probability() > 0.99);
    }
}
