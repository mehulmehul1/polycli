//! Feature Export
//!
//! Export PMXT archive data to feature parquet.

use super::{CryptoBinaryMarketSpec, FeatureRow, LabelRow, Labeler, LabelConfig, ResearchConfig};
use anyhow::{anyhow, Result};
use parquet::file::writer::SerializedFileWriter;
use std::path::Path;

/// Feature exporter
pub struct FeatureExporter {
    config: ResearchConfig,
    labeler: Labeler,
}

impl FeatureExporter {
    pub fn new(config: ResearchConfig) -> Self {
        Self {
            labeler: Labeler::new(LabelConfig::default()),
            config,
        }
    }

    /// Export features from PMXT archive
    ///
    /// # Arguments
    /// * `input_path` - Path to PMXT archive parquet
    /// * `output_path` - Output path for features parquet
    /// * `manifest_path` - Output path for manifest JSON
    pub fn export<P: AsRef<Path>>(
        &self,
        input_path: P,
        output_path: P,
        manifest_path: P,
    ) -> Result<(usize, usize)> {
        // Read PMXT archive
        let rows = self.read_pmxt_archive(input_path.as_ref())?;

        if rows.len() < self.config.min_samples {
            return Err(anyhow!(
                "Not enough samples: {} < {}",
                rows.len(),
                self.config.min_samples
            ));
        }

        // Compute labels if configured
        let labels = self.labeler.compute_labels(&rows);

        // Write features parquet
        let feature_count = self.write_features_parquet(&rows, output_path.as_ref())?;

        // Write manifest
        self.write_manifest(&rows, &labels, manifest_path.as_ref())?;

        Ok((feature_count, labels.len()))
    }

    fn read_pmxt_archive(&self, path: &Path) -> Result<Vec<FeatureRow>> {
        use parquet::file::reader::{FileReader, SerializedFileReader};

        let file = std::fs::File::open(path)?;
        let reader = SerializedFileReader::new(file)?;

        let mut rows = Vec::new();
        let iter = reader.into_row_iter(None, None, None)?;

        for row in iter {
            // Parse PMXT row into FeatureRow
            // This is a simplified implementation - actual PMXT schema would be more complex
            let condition_id = row.get_string(0).unwrap_or_default();
            let ts = row.get_long(1).unwrap_or(0);

            let yes_bid = row.get_double(2).unwrap_or(0.5);
            let yes_ask = row.get_double(3).unwrap_or(0.5);
            let no_bid = row.get_double(4).unwrap_or(0.5);
            let no_ask = row.get_double(5).unwrap_or(0.5);

            let yes_mid = (yes_bid + yes_ask) / 2.0;
            let no_mid = (no_bid + no_ask) / 2.0;

            // Parse market spec
            let spec = CryptoBinaryMarketSpec::from_slug(
                &format!("{}-market", condition_id),
                &condition_id,
                ts - 100,
                ts + 200,
            );

            let (asset, duration, family) = match spec {
                Some(s) => (s.asset.as_str().to_string(), s.duration.as_str().to_string(), s.family.as_str().to_string()),
                None => ("unknown".to_string(), "5m".to_string(), "updown_open_close".to_string()),
            };

            rows.push(FeatureRow {
                schema_version: FeatureRow::schema_version().to_string(),
                condition_id,
                market_slug: format!("{}-market", condition_id),
                instrument: condition_id.clone(),
                asset,
                duration,
                market_family: family,
                market_start_ts: ts - 100,
                market_end_ts: ts + 200,
                ts,
                yes_bid,
                yes_ask,
                no_bid,
                no_ask,
                yes_mid,
                no_mid,
                yes_spread: yes_ask - yes_bid,
                no_spread: no_ask - no_bid,
                book_sum: yes_ask + no_ask,
                book_gap: yes_ask + no_ask - 1.0,
                yes_bid_depth_1: None,
                yes_ask_depth_1: None,
                no_bid_depth_1: None,
                no_ask_depth_1: None,
                age_s: 0,
                time_remaining_s: 200,
                in_entry_window: true,
                in_exit_only_window: false,
                yes_mid_ret_1s: 0.0,
                yes_mid_ret_5s: 0.0,
                yes_mid_ret_15s: 0.0,
                yes_mid_ret_30s: 0.0,
                no_mid_ret_1s: 0.0,
                no_mid_ret_5s: 0.0,
                no_mid_ret_15s: 0.0,
                no_mid_ret_30s: 0.0,
                book_gap_z_30s: 0.0,
                yes_spread_ema_15s: 0.0,
                no_spread_ema_15s: 0.0,
                realized_vol_15s: 0.0,
                realized_vol_30s: 0.0,
                realized_vol_60s: 0.0,
                spot_price: None,
                spot_ret_1s: None,
                spot_ret_5s: None,
                spot_ret_15s: None,
                spot_realized_vol_30s: None,
            });
        }

        Ok(rows)
    }

    fn write_features_parquet(&self, rows: &[FeatureRow], path: &Path) -> Result<usize> {
        // Create directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write as JSON for simplicity (parquet writing is complex)
        // In production, use proper parquet writer
        let json = serde_json::to_string_pretty(rows)?;
        std::fs::write(path.with_extension("json"), json)?;

        Ok(rows.len())
    }

    fn write_manifest(&self, rows: &[FeatureRow], labels: &[LabelRow], path: &Path) -> Result<()> {
        use super::Manifest;

        if rows.is_empty() {
            return Ok(());
        }

        let min_ts = rows.iter().map(|r| r.ts).min().unwrap_or(0);
        let max_ts = rows.iter().map(|r| r.ts).max().unwrap_or(0);

        // Calculate train/valid/test splits (70/15/15)
        let total_duration = max_ts - min_ts;
        let train_end = min_ts + (total_duration as f64 * 0.7) as i64;
        let valid_end = min_ts + (total_duration as f64 * 0.85) as i64;

        let manifest = Manifest {
            schema_version: Manifest::schema_version().to_string(),
            features_path: path.display().to_string(),
            asset: rows[0].asset.clone(),
            duration: rows[0].duration.clone(),
            market_family: rows[0].market_family.clone(),
            start_ts: min_ts,
            end_ts: max_ts,
            resolution: format!("{}s", self.config.resolution_seconds),
            source_inputs: vec!["pmxt_archive".to_string()],
            train_start_ts: min_ts,
            train_end_ts: train_end,
            valid_start_ts: train_end,
            valid_end_ts: valid_end,
            test_start_ts: valid_end,
            test_end_ts: max_ts,
            label_columns: vec![
                "label_reachable_yes_30s".to_string(),
                "label_reachable_no_30s".to_string(),
                "label_edge_yes_30s".to_string(),
                "label_edge_no_30s".to_string(),
            ],
            feature_columns: vec![
                "yes_bid".to_string(),
                "yes_ask".to_string(),
                "no_bid".to_string(),
                "no_ask".to_string(),
                "book_sum".to_string(),
                "book_gap".to_string(),
            ],
        };

        let json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(path, json)?;

        Ok(())
    }
}

impl Default for FeatureExporter {
    fn default() -> Self {
        Self::new(ResearchConfig::default())
    }
}

/// Inspect features from a file
pub fn inspect_features<P: AsRef<Path>>(path: P, sample_count: usize) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let rows: Vec<FeatureRow> = serde_json::from_str(&content)?;

    println!("Total rows: {}", rows.len());
    println!("\nFirst {} samples:\n", sample_count.min(rows.len()));

    for (i, row) in rows.iter().take(sample_count).enumerate() {
        println!(
            "[{}] {} @ {} - yes_mid: {:.4}, book_gap: {:.4}",
            i, row.condition_id, row.ts, row.yes_mid, row.book_gap
        );
    }

    Ok(())
}
