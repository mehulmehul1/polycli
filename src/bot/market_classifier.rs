//! Market Classifier for Time-to-Expiry Based Strategy Adaptation
//!
//! Categorizes markets by time horizon and provides parameter
//! adjustments suitable for each market type.

/// Market time horizon classification
///
/// Determines strategy parameters based on how much time remains
/// until market resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketHorizon {
    /// Ultra-short markets: < 15 minutes
    ///
    /// Characteristics:
    /// - Insufficient time for jump calibration
    /// - Microstructure noise dominates signal
    /// - High turnover → fee impact
    /// - Need higher edge threshold
    UltraShort,

    /// Short markets: 15 minutes - 1 hour
    ///
    /// Characteristics:
    /// - Limited calibration data
    /// - Some noise filtering benefit
    /// - Moderate edge required
    Short,

    /// Medium markets: 1 hour - 1 day
    ///
    /// Characteristics:
    /// - Sufficient data for EM calibration
    /// - Kalman filtering beneficial
    /// - Standard edge requirements
    Medium,

    /// Long markets: > 1 day
    ///
    /// Characteristics:
    /// - Full model with EM calibration
    /// - Martingale property dominates
    /// - Lower edge threshold (more opportunities)
    /// - Fees amortized over longer duration
    Long,
}

impl MarketHorizon {
    /// Get display name for this horizon
    pub fn name(&self) -> &'static str {
        match self {
            Self::UltraShort => "ULTRA-SHORT",
            Self::Short => "SHORT",
            Self::Medium => "MEDIUM",
            Self::Long => "LONG",
        }
    }

    /// Get recommended edge multiplier for this horizon
    ///
    /// Returns a multiplier to apply to the base edge threshold.
    /// Higher multiplier = more selective trading.
    pub fn edge_multiplier(&self) -> f64 {
        match self {
            // Ultra-short: Double edge requirement (fewer but better trades)
            Self::UltraShort => 2.0,
            // Short: 1.5x edge
            Self::Short => 1.5,
            // Medium: Standard edge
            Self::Medium => 1.0,
            // Long: Lower edge (more opportunities, time for edge to develop)
            Self::Long => 0.5,
        }
    }

    /// Get whether jump detection should be enabled
    ///
    /// Ultra-short markets don't have enough data for reliable jump detection.
    pub fn enable_jump_detection(&self) -> bool {
        !matches!(self, Self::UltraShort)
    }

    /// Get EM calibration interval (observations between calibrations)
    pub fn em_calibration_interval(&self) -> usize {
        match self {
            Self::UltraShort => 20,   // More frequent but less data
            Self::Short => 30,
            Self::Medium => 50,
            Self::Long => 100,        // Less frequent, more stable
        }
    }

    /// Get minimum observations before EM calibration
    pub fn min_em_observations(&self) -> usize {
        match self {
            Self::UltraShort => 20,
            Self::Short => 30,
            Self::Medium => 50,
            Self::Long => 100,
        }
    }

    /// Get Kalman gain adjustment
    ///
    /// Returns a multiplier for the Kalman gain.
    /// Lower gain = more smoothing (good for noisy ultra-short markets).
    pub fn kalman_gain_multiplier(&self) -> f64 {
        match self {
            // Ultra-short: More aggressive filtering
            Self::UltraShort => 0.5,
            Self::Short => 0.75,
            Self::Medium => 1.0,
            Self::Long => 1.0,
        }
    }

    /// Check if this horizon is suitable for trading
    ///
    /// Ultra-short markets may be fundamentally unprofitable due to fees.
    pub fn is_recommended(&self) -> bool {
        match self {
            Self::UltraShort => false,  // Not recommended
            _ => true,
        }
    }
}

/// Classify a market by time to expiry
///
/// # Arguments
/// * `time_to_expiry_s` - Time remaining until resolution (seconds)
///
/// # Returns
/// MarketHorizon classification
pub fn classify_market(time_to_expiry_s: i64) -> MarketHorizon {
    let seconds = time_to_expiry_s.max(0);

    match seconds {
        t if t < 900 => MarketHorizon::UltraShort,   // < 15 minutes
        t if t < 3600 => MarketHorizon::Short,       // 15 min - 1 hour
        t if t < 86400 => MarketHorizon::Medium,     // 1 hour - 1 day
        _ => MarketHorizon::Long,                    // > 1 day
    }
}

/// Strategy parameters adjusted for market horizon
#[derive(Debug, Clone)]
pub struct HorizonParams {
    /// Edge multiplier
    pub edge_multiplier: f64,
    /// Enable jump detection
    pub enable_jump_detection: bool,
    /// EM calibration interval
    pub em_calibration_interval: usize,
    /// Minimum EM observations
    pub min_em_observations: usize,
    /// Kalman gain multiplier
    pub kalman_gain_multiplier: f64,
    /// Whether this horizon is recommended for trading
    pub is_recommended: bool,
}

impl HorizonParams {
    /// Create parameters for a given time to expiry
    pub fn for_time_remaining(seconds: i64) -> Self {
        let horizon = classify_market(seconds);
        Self::from_horizon(horizon)
    }

    /// Create parameters from a horizon classification
    pub fn from_horizon(horizon: MarketHorizon) -> Self {
        Self {
            edge_multiplier: horizon.edge_multiplier(),
            enable_jump_detection: horizon.enable_jump_detection(),
            em_calibration_interval: horizon.em_calibration_interval(),
            min_em_observations: horizon.min_em_observations(),
            kalman_gain_multiplier: horizon.kalman_gain_multiplier(),
            is_recommended: horizon.is_recommended(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_ultra_short() {
        let horizon = classify_market(600); // 10 minutes
        assert_eq!(horizon, MarketHorizon::UltraShort);
    }

    #[test]
    fn test_classify_short() {
        let horizon = classify_market(1800); // 30 minutes
        assert_eq!(horizon, MarketHorizon::Short);
    }

    #[test]
    fn test_classify_medium() {
        let horizon = classify_market(7200); // 2 hours
        assert_eq!(horizon, MarketHorizon::Medium);
    }

    #[test]
    fn test_classify_long() {
        let horizon = classify_market(172800); // 2 days
        assert_eq!(horizon, MarketHorizon::Long);
    }

    #[test]
    fn test_classify_boundary_conditions() {
        // 899 seconds -> UltraShort
        assert_eq!(classify_market(899), MarketHorizon::UltraShort);
        // 900 seconds -> Short
        assert_eq!(classify_market(900), MarketHorizon::Short);

        // 3599 seconds -> Short
        assert_eq!(classify_market(3599), MarketHorizon::Short);
        // 3600 seconds -> Medium
        assert_eq!(classify_market(3600), MarketHorizon::Medium);

        // 86399 seconds -> Medium
        assert_eq!(classify_market(86399), MarketHorizon::Medium);
        // 86400 seconds -> Long
        assert_eq!(classify_market(86400), MarketHorizon::Long);
    }

    #[test]
    fn test_negative_time_clamped_to_zero() {
        assert_eq!(classify_market(-100), MarketHorizon::UltraShort);
    }

    #[test]
    fn test_horizon_names() {
        assert_eq!(MarketHorizon::UltraShort.name(), "ULTRA-SHORT");
        assert_eq!(MarketHorizon::Short.name(), "SHORT");
        assert_eq!(MarketHorizon::Medium.name(), "MEDIUM");
        assert_eq!(MarketHorizon::Long.name(), "LONG");
    }

    #[test]
    fn test_edge_multipliers() {
        assert_eq!(MarketHorizon::UltraShort.edge_multiplier(), 2.0);
        assert_eq!(MarketHorizon::Short.edge_multiplier(), 1.5);
        assert_eq!(MarketHorizon::Medium.edge_multiplier(), 1.0);
        assert_eq!(MarketHorizon::Long.edge_multiplier(), 0.5);
    }

    #[test]
    fn test_jump_detection_enabled() {
        assert!(!MarketHorizon::UltraShort.enable_jump_detection());
        assert!(MarketHorizon::Short.enable_jump_detection());
        assert!(MarketHorizon::Medium.enable_jump_detection());
        assert!(MarketHorizon::Long.enable_jump_detection());
    }

    #[test]
    fn test_horizon_recommended() {
        assert!(!MarketHorizon::UltraShort.is_recommended());
        assert!(MarketHorizon::Short.is_recommended());
        assert!(MarketHorizon::Medium.is_recommended());
        assert!(MarketHorizon::Long.is_recommended());
    }

    #[test]
    fn test_horizon_params_for_time_remaining() {
        let params = HorizonParams::for_time_remaining(600); // 10 min

        assert_eq!(params.edge_multiplier, 2.0);
        assert!(!params.enable_jump_detection);
        assert!(!params.is_recommended);
    }

    #[test]
    fn test_horizon_params_from_horizon() {
        let params = HorizonParams::from_horizon(MarketHorizon::Long);

        assert_eq!(params.edge_multiplier, 0.5);
        assert!(params.enable_jump_detection);
        assert!(params.is_recommended);
        assert_eq!(params.em_calibration_interval, 100);
        assert_eq!(params.min_em_observations, 100);
    }

    #[test]
    fn test_em_calibration_intervals() {
        assert_eq!(MarketHorizon::UltraShort.em_calibration_interval(), 20);
        assert_eq!(MarketHorizon::Short.em_calibration_interval(), 30);
        assert_eq!(MarketHorizon::Medium.em_calibration_interval(), 50);
        assert_eq!(MarketHorizon::Long.em_calibration_interval(), 100);
    }

    #[test]
    fn test_kalman_gain_multipliers() {
        assert_eq!(MarketHorizon::UltraShort.kalman_gain_multiplier(), 0.5);
        assert_eq!(MarketHorizon::Short.kalman_gain_multiplier(), 0.75);
        assert_eq!(MarketHorizon::Medium.kalman_gain_multiplier(), 1.0);
        assert_eq!(MarketHorizon::Long.kalman_gain_multiplier(), 1.0);
    }
}
