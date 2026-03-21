//! Fair Value Strategy Engine
//!
//! Risk-Neutral Logit Jump-Diffusion model for edge trading.
//! Uses the paper arxiv:2510.15205 "Toward Black–Scholes for Prediction Markets".

use super::{
    Confidence, Direction, EntryReason, ExitReason, Observation, SignalSource, StrategyDecision,
    StrategyEngine, StrategyMode,
};
use crate::bot::market_classifier::{classify_market, HorizonParams, MarketHorizon};
use crate::bot::pricing::{
    jump_compensator, risk_neutral_drift, start_rtds_poller, CalibratedFairValue,
    CompositeSpotFeed, DerivedSpotFeed, EMState, FairValueModel, FilteredState, JumpCalibrator,
    KalmanFilter, LogitJumpDiffusion, LogitObservation, PolymarketRtdsFeed, SharedSpotFeed,
    SpotFeed, VolatilityCalculator,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

/// Fair value strategy signal configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FairValueSignalConfig {
    /// Minimum edge to enter a position
    pub min_edge: f64,
    /// Maximum edge for confidence capping
    pub max_edge: f64,
    /// Minimum time remaining to enter (seconds)
    pub min_time_remaining: i64,
    /// Maximum probability to enter (avoid extremes)
    pub max_entry_prob: f64,
    /// Minimum probability to enter (avoid extremes)
    pub min_entry_prob: f64,
    /// Use external spot feed if available
    pub use_external_spot: bool,
    /// Enable jump detection filtering
    pub enable_jump_detection: bool,
    /// Jump detection threshold (0-1)
    pub jump_threshold: f64,
    /// Use Kalman filtering for noise reduction
    pub use_kalman_filter: bool,
    /// Enable horizon-based parameter adaptation
    pub enable_horizon_adaptation: bool,
}

impl Default for FairValueSignalConfig {
    fn default() -> Self {
        Self {
            min_edge: 0.05, // 5% minimum edge = 2.5% after fees
            max_edge: 0.10,
            min_time_remaining: 30,
            max_entry_prob: 0.90,
            min_entry_prob: 0.10,
            use_external_spot: true,
            enable_jump_detection: true,
            jump_threshold: 0.7,
            use_kalman_filter: true,
            enable_horizon_adaptation: true,
        }
    }
}

/// Fair value strategy engine
///
/// Integrates:
/// - LogitJumpDiffusion for risk-neutral fair value
/// - KalmanFilter for microstructure noise reduction
/// - JumpCalibrator for EM-based jump parameter estimation
/// - Horizon-based parameter adaptation
pub struct FairValueEngine {
    mode: StrategyMode,
    /// Legacy Black-Scholes model (for fallback/comparison)
    fair_value: CalibratedFairValue,
    /// Logit jump-diffusion model
    logit_model: LogitJumpDiffusion,
    /// Kalman filter for noise reduction
    kalman: Option<KalmanFilter>,
    /// Jump calibrator for EM estimation
    jump_calibrator: Option<JumpCalibrator>,
    /// Spot feed for underlying price
    spot_feed: Option<CompositeSpotFeed>,
    rtds_poller: Option<tokio::task::JoinHandle<()>>,
    vol_calculator: VolatilityCalculator,
    config: FairValueSignalConfig,
    spot_at_open: Option<f64>,
    /// Previous logit for jump detection
    prev_logit: Option<f64>,
    /// Current market horizon
    current_horizon: Option<MarketHorizon>,
}

impl FairValueEngine {
    /// Create a new FairValue engine
    pub fn new(config: FairValueSignalConfig) -> Self {
        let fv_model = FairValueModel::default();
        let calibrated = CalibratedFairValue::with_defaults(fv_model);
        let logit_model = LogitJumpDiffusion::new();

        Self {
            mode: StrategyMode::FairValue,
            fair_value: calibrated,
            logit_model,
            kalman: None,
            jump_calibrator: None,
            spot_feed: None,
            rtds_poller: None,
            vol_calculator: VolatilityCalculator::new(),
            config,
            spot_at_open: None,
            prev_logit: None,
            current_horizon: None,
        }
    }

    /// Set up Polymarket RTDS feed with automatic polling
    pub fn with_rtds_feed(mut self, asset: &str) -> Self {
        let rtds_arc = Arc::new(TokioMutex::new(PolymarketRtdsFeed::new(asset)));
        let poller = start_rtds_poller(rtds_arc.clone());

        let rtds_shared = SharedSpotFeed::new(rtds_arc, "polymarket_rtds");
        let derived = DerivedSpotFeed::new(60);

        let composite = CompositeSpotFeed::new()
            .with_primary(Box::new(rtds_shared))
            .with_secondary(derived);

        self.rtds_poller = Some(poller);
        self.spot_feed = Some(composite);
        self
    }

    /// Set external spot feed (composite)
    pub fn with_spot_feed(mut self, feed: CompositeSpotFeed) -> Self {
        self.spot_feed = Some(feed);
        self
    }

    /// Get the current spot price from external feed
    fn get_spot_price(&self, _obs: &Observation) -> Option<f64> {
        if self.config.use_external_spot {
            if let Some(feed) = &self.spot_feed {
                if let Some(price) = feed.get_price() {
                    // BTC specific safety: prices in 0-1 range are likely market probabilities, not BTC/USD
                    if price > 1000.0 {
                        return Some(price);
                    }
                }
            }
        }
        None
    }

    /// Update spot at open (strike price) from external feed
    fn update_spot_at_open(&mut self, obs: &Observation) {
        if self.spot_at_open.is_none() {
            if let Some(price) = self.get_spot_price(obs) {
                println!("[FAIRVALUE] Valid Strike Captured: {:.2}", price);
                self.spot_at_open = Some(price);
            } else if obs.ts % 30 == 0 {
                println!("[FAIRVALUE] Waiting for valid BTC/USD feed to set strike...");
            }
        }
    }

    /// Initialize horizon-dependent components
    fn init_horizon_components(&mut self, obs: &Observation) {
        let horizon = classify_market(obs.time_remaining_s);

        // Re-initialize if horizon changed
        if self.current_horizon != Some(horizon) {
            self.current_horizon = Some(horizon);
            let params = HorizonParams::from_horizon(horizon);

            // Log horizon change
            if obs.ts % 60 == 0 {
                println!(
                    "[FAIRVALUE] Market horizon: {} (T={}s) | Edge mult: {:.1} | Jump detection: {} | Recommended: {}",
                    horizon.name(),
                    obs.time_remaining_s,
                    params.edge_multiplier,
                    params.enable_jump_detection,
                    params.is_recommended
                );
            }

            // Initialize Kalman filter if enabled
            if self.config.use_kalman_filter {
                // Reset Kalman with current logit estimate
                let current_logit = self.logit_model.state().logit;
                self.kalman = Some(KalmanFilter::new(current_logit));
            }

            // Initialize jump calibrator tuned for horizon
            if self.config.enable_jump_detection && params.enable_jump_detection {
                self.jump_calibrator = Some(JumpCalibrator::for_horizon(obs.time_remaining_s));
            } else {
                self.jump_calibrator = None;
            }
        }
    }

    /// Update filtered state using Kalman filter
    fn update_filtered_state(&mut self, obs: &LogitObservation) -> LocalFilteredState {
        if self.config.use_kalman_filter {
            if let Some(kalman) = &mut self.kalman {
                // Get current model parameters for prediction step
                let state = self.logit_model.state();

                // Calculate drift from current jump parameters
                let jump_intensity = self
                    .jump_calibrator
                    .as_ref()
                    .map(|c| c.jump_params().0)
                    .unwrap_or(0.1);
                let jump_second_moment = self
                    .jump_calibrator
                    .as_ref()
                    .map(|c| c.jump_params().1)
                    .unwrap_or(0.3);

                let jc = crate::bot::pricing::jump_compensator(jump_intensity, jump_second_moment);
                let drift = crate::bot::pricing::risk_neutral_drift(state.logit, state.vol, jc);

                // Predict step (forward propagation)
                let dt = 5.0 / (365.25 * 24.0 * 3600.0); // 5 seconds in years
                kalman.predict(dt, drift, state.vol);

                // Update step (incorporate measurement)
                kalman.update(obs.logit, obs.spread);

                // Create filtered state from Kalman output
                let filtered_logit = kalman.state();
                LocalFilteredState {
                    logit: filtered_logit,
                    prob: kalman.probability(),
                    vol: state.vol,
                }
            } else {
                // Fallback to simple exponential filter
                let filtered = self.logit_model.update(obs);
                LocalFilteredState {
                    logit: filtered.logit,
                    prob: filtered.prob,
                    vol: filtered.vol,
                }
            }
        } else {
            let filtered = self.logit_model.update(obs);
            LocalFilteredState {
                logit: filtered.logit,
                prob: filtered.prob,
                vol: filtered.vol,
            }
        }
    }

    /// Check if recent observation was a jump
    fn is_jump_event(&self, logit_value: f64, dt: f64) -> bool {
        if !self.config.enable_jump_detection {
            return false;
        }

        if let Some(calibrator) = &self.jump_calibrator {
            return calibrator.is_jump(logit_value, dt, self.config.jump_threshold);
        }

        // Fallback: use EMState defaults
        let em = EMState::new();
        em.is_jump(logit_value, dt, self.config.jump_threshold)
    }

    /// Check exit conditions
    fn check_exit(&self, obs: &Observation) -> Option<ExitReason> {
        let time_remaining = obs.time_remaining_s;

        // Price-based exits (Extreme repricing - take profit or stop loss)
        if obs.yes_mid > self.config.max_entry_prob || obs.yes_mid < self.config.min_entry_prob {
            return Some(ExitReason::TakeProfit { pnl_pct: 0.0 });
        }

        // Only force time exit at very end - give losing positions recovery chance
        if time_remaining < 15 {
            return Some(ExitReason::TimeExpiry {
                seconds_remaining: time_remaining,
            });
        }

        None
    }
}

impl StrategyEngine for FairValueEngine {
    fn decide(&mut self, obs: &Observation) -> StrategyDecision {
        self.update_spot_at_open(obs);

        // Initialize horizon-dependent components
        self.init_horizon_components(obs);

        // Get horizon parameters
        let horizon = self
            .current_horizon
            .unwrap_or(classify_market(obs.time_remaining_s));
        let params = HorizonParams::from_horizon(horizon);

        // 1. SAFETY: Do not trade without valid strike/spot
        let Some(strike) = self.spot_at_open else {
            return StrategyDecision::Hold;
        };

        let Some(spot) = self.get_spot_price(obs) else {
            return StrategyDecision::Hold;
        };

        // 2. Create observation and update jump calibrator
        let logit_obs = LogitObservation::from_market(
            obs.ts * 1000,
            obs.yes_bid,
            obs.yes_ask,
            1000.0, // volume proxy
        );

        // Update jump parameters if calibrator is active
        if let Some(calibrator) = &mut self.jump_calibrator {
            let (lambda, e_z2) = calibrator.update(&logit_obs);
            self.logit_model.calibrate_jumps(lambda, e_z2);
        }

        // 3. Update filtered state (Kalman or exponential)
        let filtered_state = self.update_filtered_state(&logit_obs);

        // 4. REGIME DETECTION (Bollinger Band Width)
        let bb_width = obs.indicator_5s.bb_width.unwrap_or(0.5);
        let mut min_edge = self.config.min_edge * params.edge_multiplier;
        let mut regime = "NORMAL";

        if bb_width < 0.15 {
            regime = "TIGHT";
        } else if bb_width > 0.90 {
            regime = "REVERSAL";
            min_edge *= 2.0; // Double the edge requirement in panic markets
        }

        // 5. Calculate fair probability using Logit Jump-Diffusion
        let time_remaining = obs.time_remaining_s;
        let fair_prob_result = self.logit_model.fair_prob(time_remaining);
        let fair_prob = fair_prob_result.expected;

        // 6. JUMP DETECTION - Skip entry during crash events
        if params.enable_jump_detection {
            let logit_change = (logit_obs.logit - self.prev_logit.unwrap_or(logit_obs.logit)).abs();
            let dt = 5.0; // 5 seconds
            if self.is_jump_event(logit_change, dt) {
                if obs.ts % 10 == 0 {
                    println!(
                        "[FAIRVALUE] Jump detected! Δlogit={:.3} - Blocking entry",
                        logit_change
                    );
                }
                self.prev_logit = Some(logit_obs.logit);
                return StrategyDecision::Hold;
            }
        }
        self.prev_logit = Some(logit_obs.logit);

        // 7. Calculate Edge vs ASK Price (with trading costs)
        const TRADING_COST: f64 = 0.025; // 2.5% Polymarket fee
        let edge_yes = fair_prob - obs.yes_ask - TRADING_COST;
        let edge_no = (1.0 - fair_prob) - obs.no_ask - TRADING_COST;

        // 8. MOMENTUM GATE
        let velocity = obs.indicator_5s.momentum_slope.unwrap_or(0.0);
        let ema3 = obs.indicator_5s.ema3.unwrap_or(obs.yes_mid);
        let ema6 = obs.indicator_5s.ema6.unwrap_or(obs.yes_mid);

        let momentum_allows_yes =
            regime == "TIGHT" || (velocity > -0.002 && ema3 >= (ema6 - 0.002));
        let momentum_allows_no =
            regime == "TIGHT" || velocity < 0.0 || (velocity <= 0.003 && ema3 <= (ema6 + 0.002));

        // Log decision info
        if obs.ts % 10 == 0 {
            let horizon_name = horizon.name();
            let filtered_prob = filtered_state.prob;
            println!(
                "[FAIRVALUE] {} | spot={:.1} strike={:.1} | FV={:.3} (filtered={:.3}) | EdgeY={:.3} EdgeN={:.3} | EdgeReq={:.3} | vel={:.4} regime={}",
                horizon_name, spot, strike, fair_prob, filtered_prob, edge_yes, edge_no, min_edge, velocity, regime
            );
        }

        // Check exit conditions
        if let Some(reason) = self.check_exit(obs) {
            return StrategyDecision::Exit {
                position_id: obs.condition_id.clone(),
                reason,
            };
        }

        // 9. ENTRY DECISIONS
        // YES Entry
        if edge_yes >= min_edge && momentum_allows_yes {
            let confidence = (edge_yes / self.config.max_edge).min(1.0);
            return StrategyDecision::Enter {
                direction: Direction::Yes,
                reason: EntryReason {
                    source: SignalSource::FairValue,
                    confidence: Confidence::new(confidence),
                    detail: format!(
                        "edge_yes={:.3} fv={:.3} vel={:.4} horizon={}",
                        edge_yes,
                        fair_prob,
                        velocity,
                        horizon.name()
                    ),
                    fair_value_edge: Some(edge_yes),
                    qlib_score: obs.qlib_score,
                },
            };
        }

        // NO Entry
        if edge_no >= min_edge && momentum_allows_no {
            let confidence = (edge_no / self.config.max_edge).min(1.0);
            return StrategyDecision::Enter {
                direction: Direction::No,
                reason: EntryReason {
                    source: SignalSource::FairValue,
                    confidence: Confidence::new(confidence),
                    detail: format!(
                        "edge_no={:.3} fv={:.3} vel={:.4} horizon={}",
                        edge_no,
                        fair_prob,
                        velocity,
                        horizon.name()
                    ),
                    fair_value_edge: Some(edge_no),
                    qlib_score: obs.qlib_score,
                },
            };
        }

        StrategyDecision::Hold
    }

    fn reset(&mut self) {
        // Reset components when switching markets
        self.logit_model = LogitJumpDiffusion::new();
        self.kalman = None;
        if let Some(calibrator) = &mut self.jump_calibrator {
            calibrator.reset();
        }
        self.prev_logit = None;
        self.current_horizon = None;
    }
}

impl Drop for FairValueEngine {
    fn drop(&mut self) {
        if let Some(poller) = self.rtds_poller.take() {
            poller.abort();
        }
    }
}

/// Filtered state from the model (for logging/diagnostics)
#[derive(Debug, Clone)]
struct LocalFilteredState {
    logit: f64,
    prob: f64,
    vol: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::indicators::IndicatorState;

    fn mock_obs(yes_mid: f64, yes_ask: f64, no_ask: f64, time_remaining: i64) -> Observation {
        Observation {
            ts: 1000,
            condition_id: "test".to_string(),
            market_slug: "test-market".to_string(),
            yes_bid: yes_mid - 0.01,
            yes_ask,
            no_bid: (1.0 - yes_mid) - 0.01,
            no_ask,
            yes_mid,
            no_mid: 1.0 - yes_mid,
            book_sum: yes_ask + no_ask,
            time_remaining_s: time_remaining,
            indicator_5s: IndicatorState::default(),
            indicator_1m: IndicatorState::default(),
            fair_value_prob: None,
            qlib_score: None,
        }
    }

    #[test]
    fn test_fairvalue_engine_holds_without_edge() {
        let mut engine = FairValueEngine::new(FairValueSignalConfig::default());

        // Fairly priced market - no edge
        let obs = mock_obs(0.50, 0.51, 0.51, 300);
        let decision = engine.decide(&obs);

        assert!(matches!(decision, StrategyDecision::Hold));
    }

    #[test]
    fn test_fairvalue_engine_respects_horizon_classification() {
        let mut engine = FairValueEngine::new(FairValueSignalConfig::default());

        // Ultra-short market
        let obs = mock_obs(0.50, 0.51, 0.51, 600); // 10 minutes
        engine.decide(&obs);

        assert_eq!(engine.current_horizon, Some(MarketHorizon::UltraShort));
    }

    #[test]
    fn test_fairvalue_engine_resets_state() {
        let mut engine = FairValueEngine::new(FairValueSignalConfig::default());

        engine.reset();

        assert!(engine.prev_logit.is_none());
        assert!(engine.current_horizon.is_none());
    }

    #[test]
    fn test_filtered_state_creation() {
        let state = LocalFilteredState {
            logit: 0.0,
            prob: 0.5,
            vol: 0.8,
        };

        assert_eq!(state.logit, 0.0);
        assert_eq!(state.prob, 0.5);
        assert_eq!(state.vol, 0.8);
    }
}
