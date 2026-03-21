//! Jump Calibrator using EM Algorithm
//!
//! Wrapper around EMState that maintains observation history
//! and periodically runs EM convergence to estimate jump parameters.
//!
//! This module bridges the gap between the raw EM estimator and
//! the live trading system by managing history, triggering calibration,
//! and providing jump detection for entry filtering.

use crate::bot::pricing::em_estimator::EMState;
use crate::bot::pricing::logit_model::LogitObservation;
use std::collections::VecDeque;

/// Jump calibrator with automatic EM convergence
///
/// Maintains a sliding window of recent observations and periodically
/// runs the EM algorithm to estimate jump intensity and variance.
#[derive(Debug, Clone)]
pub struct JumpCalibrator {
    /// EM algorithm state
    em: EMState,
    /// Observation history for EM calibration
    history: VecDeque<LogitObservation>,
    /// Maximum history size (tunable based on market type)
    max_history: usize,
    /// Minimum observations before running EM
    min_observations: usize,
    /// Update interval (run EM every N observations)
    update_interval: usize,
    /// Observation counter
    observation_count: usize,
    /// Previous logit for jump detection
    prev_logit: Option<f64>,
}

impl JumpCalibrator {
    /// Create a new jump calibrator
    ///
    /// # Arguments
    /// * `max_history` - Maximum number of observations to keep (default: 500)
    /// * `min_observations` - Minimum observations before EM runs (default: 50)
    /// * `update_interval` - Run EM every N observations (default: 50)
    pub fn new(max_history: usize, min_observations: usize, update_interval: usize) -> Self {
        Self {
            em: EMState::new(),
            history: VecDeque::with_capacity(max_history),
            max_history,
            min_observations,
            update_interval,
            observation_count: 0,
            prev_logit: None,
        }
    }

    /// Create with default parameters
    pub fn with_defaults() -> Self {
        Self::new(500, 50, 50)
    }

    /// Create with parameters tuned for market horizon
    pub fn for_horizon(horizon_seconds: i64) -> Self {
        match horizon_seconds {
            t if t < 900 => {
                // Ultra-short markets (< 15 min): Less history, faster updates
                Self::new(100, 20, 20)
            }
            t if t < 3600 => {
                // Short markets (15 min - 1 hour): Moderate history
                Self::new(200, 30, 30)
            }
            t if t < 86400 => {
                // Medium markets (1 hour - 1 day): Standard history
                Self::new(500, 50, 50)
            }
            _ => {
                // Long markets (> 1 day): More history for stable calibration
                Self::new(1000, 100, 100)
            }
        }
    }

    /// Update with a new observation
    ///
    /// Returns the current jump parameters (lambda, E[z²]).
    /// Periodically runs EM convergence if we have enough data.
    ///
    /// # Arguments
    /// * `obs` - New market observation
    ///
    /// # Returns
    /// (jump_intensity, jump_second_moment) for use in LogitJumpDiffusion
    pub fn update(&mut self, obs: &LogitObservation) -> (f64, f64) {
        // Store observation for EM calibration
        self.history.push_back(obs.clone());
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }

        self.observation_count += 1;

        // Run EM iteration every N observations if we have enough data
        if self.history.len() >= self.min_observations
            && self.observation_count % self.update_interval == 0
        {
            self.run_em_calibration();
        }

        // Store previous logit for jump detection
        self.prev_logit = Some(obs.logit);

        self.em.jump_params()
    }

    /// Run EM algorithm convergence on stored history
    fn run_em_calibration(&mut self) {
        let observations: Vec<_> = self.history.make_contiguous().to_vec();
        if observations.is_empty() {
            return;
        }

        // Estimate dt from timestamps (assuming regular observations)
        let dt = self.estimate_dt(&observations);

        // Run EM convergence
        let iterations = self.em.converge(&observations, dt, 100, 1e-6);

        // Log calibration results periodically
        if self.observation_count % (self.update_interval * 4) == 0 {
            let (lambda, e_z2) = self.em.jump_params();
            println!(
                "[JUMP_CALIB] EM converged in {} iters: λ={:.2e} E[z²]={:.3}",
                iterations, lambda, e_z2
            );
        }
    }

    /// Estimate time step between observations
    fn estimate_dt(&self, observations: &[LogitObservation]) -> f64 {
        if observations.len() < 2 {
            return 5.0; // Default 5 seconds
        }

        // Calculate median time difference
        let mut diffs: Vec<f64> = observations
            .windows(2)
            .map(|w| (w[1].timestamp - w[0].timestamp) as f64 / 1000.0)
            .collect();

        diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let median = if diffs.is_empty() {
            5.0
        } else {
            diffs[diffs.len() / 2]
        };

        median.max(0.1).min(3600.0) // Clamp between 0.1s and 1 hour
    }

    /// Check if a recent logit change is likely a jump
    ///
    /// Returns true if the jump probability exceeds the threshold.
    ///
    /// # Arguments
    /// * `logit_value` - Current logit value
    /// * `dt` - Time since last observation (seconds)
    /// * `threshold` - Jump probability threshold (default: 0.7)
    pub fn is_jump(&self, logit_value: f64, dt: f64, threshold: f64) -> bool {
        let Some(prev_logit) = self.prev_logit else {
            return false;
        };

        let logit_change = (logit_value - prev_logit).abs();
        self.em.is_jump(logit_change, dt, threshold)
    }

    /// Get current jump parameters without updating
    pub fn jump_params(&self) -> (f64, f64) {
        self.em.jump_params()
    }

    /// Get jump probability for a logit change
    pub fn jump_probability(&self, logit_change: f64, dt: f64) -> f64 {
        self.em.jump_probability(logit_change, dt)
    }

    /// Reset calibrator state
    pub fn reset(&mut self) {
        self.history.clear();
        self.observation_count = 0;
        self.prev_logit = None;
        self.em = EMState::new();
    }

    /// Get calibration status
    pub fn is_calibrated(&self) -> bool {
        self.history.len() >= self.min_observations
    }

    /// Get observation count
    pub fn observation_count(&self) -> usize {
        self.observation_count
    }

    /// Get history size
    pub fn history_size(&self) -> usize {
        self.history.len()
    }
}

impl Default for JumpCalibrator {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_obs(timestamp: i64, prob: f64) -> LogitObservation {
        LogitObservation {
            timestamp,
            prob,
            logit: crate::bot::pricing::logit_model::prob_to_logit(prob),
            spread: 0.01,
            volume: 1000.0,
        }
    }

    #[test]
    fn test_jump_calibrator_initialization() {
        let cal = JumpCalibrator::with_defaults();

        assert!(!cal.is_calibrated());
        assert_eq!(cal.observation_count(), 0);
        assert_eq!(cal.history_size(), 0);
    }

    #[test]
    fn test_jump_calibrator_accumulates_history() {
        let mut cal = JumpCalibrator::with_defaults();

        for i in 0..10 {
            cal.update(&mock_obs(i * 1000, 0.5));
        }

        assert_eq!(cal.history_size(), 10);
        assert_eq!(cal.observation_count(), 10);
    }

    #[test]
    fn test_jump_calibrator_max_history_enforced() {
        let mut cal = JumpCalibrator::new(5, 3, 10);

        // Add 10 observations (more than max_history)
        for i in 0..10 {
            cal.update(&mock_obs(i * 1000, 0.5));
        }

        // Should only keep last 5
        assert_eq!(cal.history_size(), 5);
    }

    #[test]
    fn test_jump_calibrator_horizon_tuning() {
        let cal_ultra = JumpCalibrator::for_horizon(600); // 10 min
        assert_eq!(cal_ultra.max_history, 100);

        let cal_short = JumpCalibrator::for_horizon(1800); // 30 min
        assert_eq!(cal_short.max_history, 200);

        let cal_medium = JumpCalibrator::for_horizon(7200); // 2 hours
        assert_eq!(cal_medium.max_history, 500);

        let cal_long = JumpCalibrator::for_horizon(172800); // 2 days
        assert_eq!(cal_long.max_history, 1000);
    }

    #[test]
    fn test_jump_detection_requires_previous() {
        let cal = JumpCalibrator::with_defaults();

        // No previous logit - should not detect jump
        assert!(!cal.is_jump(0.5, 1.0, 0.7));
    }

    #[test]
    fn test_jump_detection_with_previous() {
        let mut cal = JumpCalibrator::with_defaults();

        // Add first observation
        cal.update(&mock_obs(0, 0.5));

        // Small change - not a jump
        assert!(!cal.is_jump(0.01, 1.0, 0.7));

        // Large change - should be jump
        assert!(cal.is_jump(1.0, 1.0, 0.5));
    }

    #[test]
    fn test_jump_probability_calculation() {
        let cal = JumpCalibrator::with_defaults();

        // Small change should have low jump probability
        let jp_small = cal.jump_probability(0.01, 1.0);

        // Large change should have higher jump probability
        let jp_large = cal.jump_probability(1.0, 1.0);

        assert!(jp_large > jp_small);
    }

    #[test]
    fn test_reset_clears_state() {
        let mut cal = JumpCalibrator::with_defaults();

        for i in 0..10 {
            cal.update(&mock_obs(i * 1000, 0.5));
        }

        cal.reset();

        assert_eq!(cal.history_size(), 0);
        assert_eq!(cal.observation_count(), 0);
        assert!(!cal.is_calibrated());
    }

    #[test]
    fn test_dt_estimation() {
        let cal = JumpCalibrator::with_defaults();

        let obs = vec![
            mock_obs(0, 0.5),
            mock_obs(5000, 0.5),   // 5s
            mock_obs(10000, 0.5),  // 5s
            mock_obs(20000, 0.5),  // 10s (outlier)
        ];

        // Median should be 5.0
        let dt = cal.estimate_dt(&obs);
        assert_eq!(dt, 5.0);
    }

    #[test]
    fn test_dt_estimation_single_obs() {
        let cal = JumpCalibrator::with_defaults();

        let obs = vec![mock_obs(0, 0.5)];

        // Should return default for single observation
        let dt = cal.estimate_dt(&obs);
        assert_eq!(dt, 5.0);
    }

    #[test]
    fn test_jump_params_returned() {
        let mut cal = JumpCalibrator::with_defaults();

        let (lambda, e_z2) = cal.update(&mock_obs(0, 0.5));

        // Should return EM default values
        assert!(lambda > 0.0);
        assert!(e_z2 > 0.0);
    }
}
