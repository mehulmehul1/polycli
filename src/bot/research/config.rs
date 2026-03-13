//! Research Configuration

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Research configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ResearchConfig {
    /// Output directory for features
    pub features_dir: PathBuf,
    /// Output directory for scores
    pub scores_dir: PathBuf,
    /// Output directory for manifests
    pub manifests_dir: PathBuf,
    /// Feature resolution in seconds
    pub resolution_seconds: i64,
    /// Label horizons in seconds
    pub label_horizons: Vec<i32>,
    /// Score freshness TTL in seconds
    pub score_freshness_ttl: i64,
    /// Minimum samples for export
    pub min_samples: usize,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            features_dir: PathBuf::from("data/research/features"),
            scores_dir: PathBuf::from("data/research/scores"),
            manifests_dir: PathBuf::from("data/research/manifests"),
            resolution_seconds: 1,
            label_horizons: vec![15, 30, 45],
            score_freshness_ttl: 300,
            min_samples: 100,
        }
    }
}

impl ResearchConfig {
    pub fn new<P: Into<PathBuf>>(base: P) -> Self {
        let base = base.into();
        Self {
            features_dir: base.join("features"),
            scores_dir: base.join("scores"),
            manifests_dir: base.join("manifests"),
            ..Default::default()
        }
    }
}
