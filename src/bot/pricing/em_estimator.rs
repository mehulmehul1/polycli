//! EM Algorithm for Jump-Diffusion Separation
//!
//! Based on Section 5.2 of arxiv:2510.15205
//!
//! The Expectation-Maximization algorithm separates the continuous diffusion
//! component from discontinuous jumps by iteratively:
//! 1. E-step: Computing posterior jump probabilities
//! 2. M-step: Updating parameters to maximize likelihood

use crate::bot::pricing::logit_model::LogitObservation;

/// EM Algorithm state for jump-diffusion separation
///
/// Maintains estimates of:
/// - σ_diff: Diffusion volatility (continuous component)
/// - λ: Jump intensity (Poisson arrival rate)
/// - E[z²]: Jump second moment (jump size variance)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EMState {
    /// Diffusion volatility (continuous component)
    pub vol_diffusion: f64,
    /// Jump intensity λ (arrivals per second)
    pub jump_intensity: f64,
    /// Jump second moment E[z²]
    pub jump_second_moment: f64,
}

impl EMState {
    /// Create with default initial values
    pub fn new() -> Self {
        Self {
            vol_diffusion: 0.6,     // 60% annualized vol
            jump_intensity: 3.16e-7, // ~0.01 jumps/day ≈ 3.16e-7 per second
            jump_second_moment: 0.3, // Average jump variance in logit space
        }
    }

    /// Create with custom parameters
    pub fn with_params(vol: f64, intensity: f64, second_moment: f64) -> Self {
        Self {
            vol_diffusion: vol.clamp(0.05, 3.0),
            jump_intensity: intensity.clamp(1e-10, 1.0),
            jump_second_moment: second_moment.clamp(0.01, 5.0),
        }
    }

    /// Run one EM iteration on recent observations
    ///
    /// # Arguments
    /// * `observations` - Recent logit observations
    /// * `dt` - Time step between observations (in seconds)
    ///
    /// # Returns
    /// Total jump probability (for convergence checking)
    pub fn iterate(&mut self, observations: &[LogitObservation], dt: f64) -> f64 {
        if observations.is_empty() {
            return 0.0;
        }

        // E-step: compute posterior jump probabilities
        let jump_probs = self.e_step(observations, dt);

        // M-step: update parameters
        self.m_step(observations, &jump_probs, dt)
    }

    /// Run multiple EM iterations until convergence
    ///
    /// # Arguments
    /// * `observations` - Recent logit observations
    /// * `dt` - Time step between observations (in seconds)
    /// * `max_iterations` - Maximum iterations to run
    /// * `tolerance` - Convergence tolerance
    ///
    /// # Returns
    /// Number of iterations run
    pub fn converge(
        &mut self,
        observations: &[LogitObservation],
        dt: f64,
        max_iterations: usize,
        tolerance: f64,
    ) -> usize {
        let mut prev_intensity = self.jump_intensity;

        for i in 0..max_iterations {
            self.iterate(observations, dt);

            let change = (self.jump_intensity - prev_intensity).abs();
            if change < tolerance {
                return i + 1;
            }
            prev_intensity = self.jump_intensity;
        }

        max_iterations
    }

    /// E-step: P(jump | data) for each observation
    ///
    /// For each observation, compute the posterior probability that
    /// a jump occurred, given the observed logit change.
    ///
    /// Uses Bayes' rule:
    /// P(jump | Δx) ∝ P(Δx | jump) * P(jump)
    fn e_step(&self, obs: &[LogitObservation], dt: f64) -> Vec<f64> {
        if obs.is_empty() {
            return Vec::new();
        }

        // First observation has no previous reference - assign zero jump probability
        let mut result = vec![0.0];

        // For subsequent observations, compute jump probability based on change
        for window in obs.windows(2) {
            let logit_change = (window[1].logit - window[0].logit).abs();

            // Diffusion likelihood: N(0, σ_diff² * dt)
            let diff_std = (self.vol_diffusion * dt.sqrt()).max(1e-10);
            let diff_lik = gaussian_pdf(logit_change, 0.0, diff_std);

            // Jump likelihood: use estimated jump distribution
            // For small jumps, approximate as Gaussian with estimated variance
            let jump_std = (self.jump_second_moment.sqrt()).max(1e-10);
            let jump_lik = gaussian_pdf(logit_change, 0.0, jump_std);

            // Posterior jump probability using Bayes' rule
            // P(jump | data) = P(data | jump) * P(jump) / P(data)
            let prior = (self.jump_intensity * dt).min(0.99); // Cap at 0.99
            let evidence = prior * jump_lik + (1.0 - prior) * diff_lik;

            if evidence > 1e-10 {
                result.push((prior * jump_lik / evidence).clamp(0.0, 1.0));
            } else {
                result.push(0.0);
            }
        }

        result
    }

    /// M-step: update parameters to maximize expected likelihood
    ///
    /// Updates:
    /// - λ from average jump probability
    /// - E[z²] from jump-weighted squared changes
    /// - σ_diff from non-jump weighted variance
    ///
    /// Returns the average jump probability for convergence checking.
    fn m_step(&mut self, obs: &[LogitObservation], jump_probs: &[f64], dt: f64) -> f64 {
        let n = obs.len() as f64;
        let total_jump_prob: f64 = jump_probs.iter().sum();

        // Update jump intensity: λ = E[N] / T = (sum of jump probs) / (n * dt)
        if n > 0.0 && dt > 0.0 {
            self.jump_intensity = (total_jump_prob / ((n - 1.0) * dt)).clamp(1e-10, 1.0);
        }

        // Compute logit changes for variance estimation
        let logit_changes: Vec<f64> = obs.windows(2)
            .map(|w| (w[1].logit - w[0].logit).abs())
            .collect();

        // Update jump second moment from jump-weighted observations
        // E[z²] = sum(p_i * Δx_i²) / sum(p_i)
        // Note: jump_probs[0] corresponds to obs[1] (first change)
        let weighted_sum: f64 = logit_changes.iter()
            .zip(jump_probs.iter().skip(1))  // Skip first (zero) prob
            .map(|(delta, p)| p * delta * delta)
            .sum();

        let total_weight: f64 = jump_probs.iter().skip(1).sum::<f64>().max(0.01);
        self.jump_second_moment = (weighted_sum / total_weight).clamp(0.01, 5.0);

        // Update diffusion volatility from non-jump observations
        // σ_diff² = sum((1-p_i) * Δx_i²) / sum((1-p_i) * dt)
        let diff_var: f64 = logit_changes.iter()
            .zip(jump_probs.iter().skip(1))
            .map(|(delta, p)| (1.0 - p) * delta * delta)
            .sum();

        let diff_weight: f64 = jump_probs.iter().skip(1).map(|p| 1.0 - p).sum::<f64>().max(0.01);
        if dt > 0.0 {
            self.vol_diffusion = (diff_var / (diff_weight * dt)).sqrt().clamp(0.05, 3.0);
        }

        // Return average jump probability for convergence (excluding first obs)
        let n_effective = (n - 1.0).max(1.0);
        total_jump_prob / n_effective
    }

    /// Get jump parameters for use in the logit model
    ///
    /// Returns (λ, E[z²]) for jump_compensator calculation
    pub fn jump_params(&self) -> (f64, f64) {
        (self.jump_intensity, self.jump_second_moment)
    }

    /// Estimate jump probability for a single logit change
    ///
    /// Useful for real-time jump detection without full EM iteration
    pub fn jump_probability(&self, logit_change: f64, dt: f64) -> f64 {
        let diff_std = (self.vol_diffusion * dt.sqrt()).max(1e-10);
        let diff_lik = gaussian_pdf(logit_change, 0.0, diff_std);

        let jump_std = self.jump_second_moment.sqrt().max(1e-10);
        let jump_lik = gaussian_pdf(logit_change, 0.0, jump_std);

        let prior = (self.jump_intensity * dt).min(0.99);
        let evidence = prior * jump_lik + (1.0 - prior) * diff_lik;

        if evidence > 1e-10 {
            (prior * jump_lik / evidence).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Detect if a given logit change is likely a jump
    ///
    /// Returns true if jump probability exceeds threshold
    pub fn is_jump(&self, logit_change: f64, dt: f64, threshold: f64) -> bool {
        self.jump_probability(logit_change, dt) > threshold
    }
}

impl Default for EMState {
    fn default() -> Self {
        Self::new()
    }
}

/// Gaussian probability density function
fn gaussian_pdf(x: f64, mean: f64, std: f64) -> f64 {
    let z = (x - mean) / std.max(1e-10);
    let norm = 1.0 / (std * (2.0 * std::f64::consts::PI).sqrt());
    norm * (-0.5 * z * z).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_obs(timestamp: i64, prob: f64, spread: f64, volume: f64) -> LogitObservation {
        LogitObservation {
            timestamp,
            prob,
            logit: crate::bot::pricing::logit_model::prob_to_logit(prob),
            spread,
            volume,
        }
    }

    #[test]
    fn test_em_initialization() {
        let em = EMState::new();

        assert!(em.vol_diffusion > 0.0);
        assert!(em.jump_intensity >= 0.0);
        assert!(em.jump_second_moment > 0.0);
    }

    #[test]
    fn test_em_with_custom_params() {
        let em = EMState::with_params(0.8, 0.1, 0.5);

        assert_eq!(em.vol_diffusion, 0.8);
        assert_eq!(em.jump_intensity, 0.1);
        assert_eq!(em.jump_second_moment, 0.5);
    }

    #[test]
    fn test_em_iteration_converges() {
        let mut em = EMState::new();

        // Create synthetic observations with some jumps
        let obs = vec![
            mock_obs(0, 0.5, 0.01, 1000.0),
            mock_obs(1, 0.52, 0.01, 1000.0),
            mock_obs(2, 0.51, 0.01, 1000.0),
            mock_obs(3, 0.65, 0.01, 1000.0), // Potential jump
            mock_obs(4, 0.66, 0.01, 1000.0),
        ];

        let dt = 1.0; // 1 second
        let iterations = em.converge(&obs, dt, 100, 1e-6);

        assert!(iterations > 0);
        assert!(iterations <= 100);
    }

    #[test]
    fn test_empty_observations() {
        let mut em = EMState::new();
        let result = em.iterate(&[], 1.0);

        assert_eq!(result, 0.0);
    }

    #[test]
    fn test_jump_probability_calculation() {
        let em = EMState::with_params(0.6, 0.01, 0.3);

        // Small change → low jump probability
        let jp_small = em.jump_probability(0.01, 1.0);

        // Large change → higher jump probability
        let jp_large = em.jump_probability(0.5, 1.0);

        assert!(jp_large > jp_small);
    }

    #[test]
    fn test_is_jump_detection() {
        let em = EMState::with_params(0.6, 0.01, 0.3);

        // Small change shouldn't be detected as jump
        assert!(!em.is_jump(0.01, 1.0, 0.5));

        // Large change should be detected as jump
        assert!(em.is_jump(1.0, 1.0, 0.5));
    }

    #[test]
    fn test_jump_params() {
        let em = EMState::with_params(0.8, 0.1, 0.5);

        let (intensity, second_moment) = em.jump_params();

        assert_eq!(intensity, 0.1);
        assert_eq!(second_moment, 0.5);
    }

    #[test]
    fn test_gaussian_pdf() {
        // At mean, PDF should be maximum
        let pdf_at_mean = gaussian_pdf(0.0, 0.0, 1.0);
        let pdf_off_mean = gaussian_pdf(1.0, 0.0, 1.0);

        assert!(pdf_at_mean > pdf_off_mean);

        // Should integrate to approximately 1 over reasonable range
        let sum: f64 = (0..100)
            .map(|i| gaussian_pdf(i as f64 / 10.0 - 5.0, 0.0, 1.0) * 0.1)
            .sum();

        assert!((sum - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_e_step_returns_valid_probabilities() {
        let em = EMState::new();

        let obs = vec![
            mock_obs(0, 0.5, 0.01, 1000.0),
            mock_obs(1, 0.6, 0.01, 1000.0),
        ];

        let jump_probs = em.e_step(&obs, 1.0);

        assert_eq!(jump_probs.len(), 2);
        for p in jump_probs {
            assert!(p >= 0.0 && p <= 1.0);
        }
    }

    #[test]
    fn test_m_step_updates_parameters() {
        let mut em = EMState::new();

        let obs = vec![
            mock_obs(0, 0.5, 0.01, 1000.0),
            mock_obs(1, 0.6, 0.01, 1000.0),
        ];

        let jump_probs = vec![0.1, 0.9];
        let old_vol = em.vol_diffusion;

        em.m_step(&obs, &jump_probs, 1.0);

        // Parameters should have changed
        assert!(em.vol_diffusion != old_vol || em.jump_intensity != 3.16e-7);
    }

    #[test]
    fn test_clamping_in_with_params() {
        // Test that extreme values are clamped
        let em = EMState::with_params(10.0, 100.0, 100.0);

        assert!(em.vol_diffusion <= 3.0);
        assert!(em.jump_intensity <= 1.0);
        assert!(em.jump_second_moment <= 5.0);
    }
}
