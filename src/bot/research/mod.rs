//! Research Module
//!
//! Infrastructure for ML research: feature export, labeling, score loading.

pub mod config;
pub mod schema;
pub mod market_spec;
pub mod feature_export;
pub mod cost_model;
pub mod labeling;
pub mod score_loader;
pub mod fusion;

pub use config::ResearchConfig;
pub use schema::{FeatureRow, LabelRow, ScoreRow, Manifest};
pub use market_spec::{CryptoBinaryMarketSpec, MarketFamily, SupportedAsset, SupportedDuration};
pub use cost_model::CostModel;
pub use labeling::{LabelConfig, Labeler};
pub use score_loader::ScoreLoader;
pub use feature_export::FeatureExporter;
pub use fusion::{FusionMode, FusionEngine, FusionConfig, FusionDecision};
