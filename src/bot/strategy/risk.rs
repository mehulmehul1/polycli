//! Risk Gate Module
//!
//! Position sizing, daily loss limits, and cooldown management.

use serde::{Deserialize, Serialize};

/// Risk gate configuration
#[derive(Debug, Clone, Deserialize)]
pub struct RiskConfig {
    /// Maximum position size in USD
    pub max_position_usd: f64,
    /// Maximum daily loss in USD
    pub max_daily_loss_usd: f64,
    /// Cooldown period after exit in seconds
    pub cooldown_seconds: u64,
    /// Maximum spread allowed for entry
    pub max_spread: f64,
    /// Minimum time remaining for entry (seconds)
    pub min_time_remaining: i64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_position_usd: 100.0,
            max_daily_loss_usd: 50.0,
            cooldown_seconds: 60,
            max_spread: 0.08,
            min_time_remaining: 60,
        }
    }
}

/// Risk gate state
#[derive(Debug, Clone, Default)]
pub struct RiskState {
    pub daily_pnl: f64,
    pub last_exit_ts: Option<i64>,
    pub position_count: u32,
}

/// Risk gate checker
pub struct RiskGate {
    config: RiskConfig,
    state: RiskState,
}

impl RiskGate {
    pub fn new(config: RiskConfig) -> Self {
        Self {
            config,
            state: RiskState::default(),
        }
    }

    /// Check if entry is allowed
    pub fn check_entry(&self, spread: f64, time_remaining_s: i64, current_ts: i64) -> Result<(), String> {
        // Check spread
        if spread > self.config.max_spread {
            return Err(format!("Spread too wide: {:.4} > {:.4}", spread, self.config.max_spread));
        }

        // Check time remaining
        if time_remaining_s < self.config.min_time_remaining {
            return Err(format!(
                "Not enough time remaining: {}s < {}s",
                time_remaining_s, self.config.min_time_remaining
            ));
        }

        // Check cooldown
        if let Some(last_exit) = self.state.last_exit_ts {
            let elapsed = current_ts - last_exit;
            if elapsed < self.config.cooldown_seconds as i64 {
                return Err(format!(
                    "Cooldown active: {}s remaining",
                    self.config.cooldown_seconds as i64 - elapsed
                ));
            }
        }

        // Check daily loss
        if self.state.daily_pnl < -self.config.max_daily_loss_usd {
            return Err(format!(
                "Daily loss limit reached: ${:.2}",
                self.state.daily_pnl
            ));
        }

        Ok(())
    }

    /// Record an exit
    pub fn record_exit(&mut self, pnl: f64, ts: i64) {
        self.state.daily_pnl += pnl;
        self.state.last_exit_ts = Some(ts);
        self.state.position_count += 1;
    }

    /// Calculate position size based on risk
    pub fn calculate_size(&self, confidence: f64) -> f64 {
        // Scale position size by confidence
        let base_size = self.config.max_position_usd * 0.5;
        base_size * confidence.min(1.0)
    }

    /// Reset daily state
    pub fn reset_daily(&mut self) {
        self.state.daily_pnl = 0.0;
        self.state.last_exit_ts = None;
        self.state.position_count = 0;
    }

    /// Get current state
    pub fn state(&self) -> &RiskState {
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_wide_spread() {
        let gate = RiskGate::new(RiskConfig::default());
        let result = gate.check_entry(0.10, 300, 0);
        assert!(result.is_err());
    }

    #[test]
    fn blocks_short_time() {
        let gate = RiskGate::new(RiskConfig::default());
        let result = gate.check_entry(0.02, 30, 0);
        assert!(result.is_err());
    }

    #[test]
    fn allows_normal_entry() {
        let gate = RiskGate::new(RiskConfig::default());
        let result = gate.check_entry(0.02, 300, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn enforces_cooldown() {
        let mut gate = RiskGate::new(RiskConfig {
            cooldown_seconds: 60,
            ..Default::default()
        });
        gate.record_exit(10.0, 1000);

        // Immediately after - blocked
        let result = gate.check_entry(0.02, 300, 1010);
        assert!(result.is_err());

        // After cooldown - allowed
        let result = gate.check_entry(0.02, 300, 1070);
        assert!(result.is_ok());
    }

    #[test]
    fn blocks_after_daily_loss() {
        let mut gate = RiskGate::new(RiskConfig {
            max_daily_loss_usd: 50.0,
            ..Default::default()
        });
        gate.record_exit(-30.0, 1000);
        gate.record_exit(-30.0, 2000);

        let result = gate.check_entry(0.02, 300, 3000);
        assert!(result.is_err());
    }
}
