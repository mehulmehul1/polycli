//! Pricing Module
//!
//! Fair value models and volatility calculations.

mod calibrated;
mod em_estimator;
mod fair_value;
mod jump_calibrator;
mod kalman_filter;
mod logit_model;
mod monitor;
mod spot_feed;
mod volatility;

pub use calibrated::{CalibratedFairValue, CalibratedProb, CalibrationConfig};
pub use em_estimator::EMState;
pub use fair_value::{FairValueConfig, FairValueModel};
pub use jump_calibrator::JumpCalibrator;
pub use kalman_filter::KalmanFilter;
pub use logit_model::{
    sigmoid, prob_to_logit, logit_to_prob, risk_neutral_drift, jump_compensator,
    LogitJumpDiffusion, LogitObservation, FilteredState, FairProbability,
};
pub use monitor::{FairValueMonitor, FairValuePrediction, MonitorSummary};
pub use spot_feed::{
    SpotFeed, ChainlinkFeed, ChainlinkConfig, DerivedSpotFeed, CompositeSpotFeed,
    PolymarketRtdsFeed, SharedSpotFeed, start_rtds_poller,
};
pub use volatility::{VolSurface, VolatilityCalculator};
