//! Research Schema
//!
//! Data structures matching docs/research_schema.md

use serde::{Deserialize, Serialize};

/// Feature row for export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRow {
    // Identity columns
    pub schema_version: String,
    pub condition_id: String,
    pub market_slug: String,
    pub instrument: String,
    pub asset: String,
    pub duration: String,
    pub market_family: String,
    pub market_start_ts: i64,
    pub market_end_ts: i64,
    pub ts: i64,

    // Book state
    pub yes_bid: f64,
    pub yes_ask: f64,
    pub no_bid: f64,
    pub no_ask: f64,
    pub yes_mid: f64,
    pub no_mid: f64,
    pub yes_spread: f64,
    pub no_spread: f64,
    pub book_sum: f64,
    pub book_gap: f64,

    // Depth (optional)
    pub yes_bid_depth_1: Option<f64>,
    pub yes_ask_depth_1: Option<f64>,
    pub no_bid_depth_1: Option<f64>,
    pub no_ask_depth_1: Option<f64>,

    // Market state
    pub age_s: i32,
    pub time_remaining_s: i32,
    pub in_entry_window: bool,
    pub in_exit_only_window: bool,

    // Derived microstructure
    pub yes_mid_ret_1s: f64,
    pub yes_mid_ret_5s: f64,
    pub yes_mid_ret_15s: f64,
    pub yes_mid_ret_30s: f64,
    pub no_mid_ret_1s: f64,
    pub no_mid_ret_5s: f64,
    pub no_mid_ret_15s: f64,
    pub no_mid_ret_30s: f64,
    pub book_gap_z_30s: f64,
    pub yes_spread_ema_15s: f64,
    pub no_spread_ema_15s: f64,
    pub realized_vol_15s: f64,
    pub realized_vol_30s: f64,
    pub realized_vol_60s: f64,

    // Optional spot columns
    pub spot_price: Option<f64>,
    pub spot_ret_1s: Option<f64>,
    pub spot_ret_5s: Option<f64>,
    pub spot_ret_15s: Option<f64>,
    pub spot_realized_vol_30s: Option<f64>,
}

impl FeatureRow {
    pub fn schema_version() -> &'static str {
        "features_v1"
    }
}

/// Label row for ML training
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelRow {
    pub condition_id: String,
    pub ts: i64,

    // Reachable profit labels
    pub label_reachable_yes_15s: i8,
    pub label_reachable_yes_30s: i8,
    pub label_reachable_yes_45s: i8,
    pub label_reachable_no_15s: i8,
    pub label_reachable_no_30s: i8,
    pub label_reachable_no_45s: i8,

    // Edge labels
    pub label_edge_yes_15s: f64,
    pub label_edge_yes_30s: f64,
    pub label_edge_yes_45s: f64,
    pub label_edge_no_15s: f64,
    pub label_edge_no_30s: f64,
    pub label_edge_no_45s: f64,

    // Adverse excursion
    pub label_adverse_yes_30s: f64,
    pub label_adverse_no_30s: f64,

    // Resolution (optional)
    pub label_resolution: Option<i8>,
}

/// Score row from Qlib model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreRow {
    pub schema_version: String,
    pub condition_id: String,
    pub ts: i64,
    pub model_version: String,

    // Scores
    pub score_yes_15s: f64,
    pub score_yes_30s: f64,
    pub score_yes_45s: f64,
    pub score_no_15s: f64,
    pub score_no_30s: f64,
    pub score_no_45s: f64,

    // Risk scores
    pub risk_yes_30s: f64,
    pub risk_no_30s: f64,

    // Freshness
    pub fresh_until_ts: i64,
}

impl ScoreRow {
    pub fn schema_version() -> &'static str {
        "scores_v1"
    }
}

/// Manifest metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: String,
    pub features_path: String,
    pub asset: String,
    pub duration: String,
    pub market_family: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub resolution: String,
    pub source_inputs: Vec<String>,
    pub train_start_ts: i64,
    pub train_end_ts: i64,
    pub valid_start_ts: i64,
    pub valid_end_ts: i64,
    pub test_start_ts: i64,
    pub test_end_ts: i64,
    pub label_columns: Vec<String>,
    pub feature_columns: Vec<String>,
}

impl Manifest {
    pub fn schema_version() -> &'static str {
        "manifest_v1"
    }
}
