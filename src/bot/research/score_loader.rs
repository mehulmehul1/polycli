//! Score Loader
//!
//! Load and validate Qlib model scores.

use super::ScoreRow;
use anyhow::{anyhow, Result};
use parquet::file::reader::{FileReader, SerializedFileReader};
use std::collections::HashMap;
use std::path::Path;

/// Score loader with caching
pub struct ScoreLoader {
    /// Path to scores directory
    scores_path: std::path::PathBuf,
    /// Cache of loaded scores by (condition_id, ts)
    cache: HashMap<String, HashMap<i64, ScoreRow>>,
    /// Freshness TTL in seconds
    freshness_ttl: i64,
}

impl ScoreLoader {
    pub fn new<P: AsRef<Path>>(path: P, freshness_ttl: i64) -> Self {
        Self {
            scores_path: path.as_ref().to_path_buf(),
            cache: HashMap::new(),
            freshness_ttl,
        }
    }

    /// Load scores from a parquet file
    pub fn load(&mut self, path: &Path) -> Result<usize> {
        let file = std::fs::File::open(path)?;
        let reader = SerializedFileReader::new(file)?;
        let iter = reader.into_row_iter(None, None, None)?;

        let mut count = 0;
        for row in iter {
            // Parse row into ScoreRow
            // This is simplified - actual implementation would use parquet schema
            let condition_id = row.get_string(1)?.to_string();
            let ts = row.get_long(2)?;

            let score = ScoreRow {
                schema_version: ScoreRow::schema_version().to_string(),
                condition_id: condition_id.clone(),
                ts,
                model_version: "v1".to_string(),
                score_yes_15s: row.get_double(4)?,
                score_yes_30s: row.get_double(5)?,
                score_yes_45s: row.get_double(6)?,
                score_no_15s: row.get_double(7)?,
                score_no_30s: row.get_double(8)?,
                score_no_45s: row.get_double(9)?,
                risk_yes_30s: row.get_double(10)?,
                risk_no_30s: row.get_double(11)?,
                fresh_until_ts: row.get_long(12)?,
            };

            self.cache
                .entry(condition_id)
                .or_insert_with(HashMap::new)
                .insert(ts, score);

            count += 1;
        }

        Ok(count)
    }

    /// Get score for a condition_id and timestamp
    pub fn get_score(&self, condition_id: &str, ts: i64) -> Option<&ScoreRow> {
        self.cache.get(condition_id)?.get(&ts)
    }

    /// Get most recent score before timestamp
    pub fn get_latest_score(&self, condition_id: &str, ts: i64) -> Option<&ScoreRow> {
        let market_scores = self.cache.get(condition_id)?;
        let mut best: Option<(&i64, &ScoreRow)> = None;

        for (score_ts, score) in market_scores.iter() {
            if *score_ts <= ts {
                if best.is_none() || *score_ts > *best.unwrap().0 {
                    best = Some((score_ts, score));
                }
            }
        }

        best.map(|(_, s)| s)
    }

    /// Check if score is fresh
    pub fn is_fresh(&self, score: &ScoreRow, current_ts: i64) -> bool {
        score.fresh_until_ts >= current_ts
    }

    /// Validate score schema version
    pub fn validate_schema(&self, score: &ScoreRow) -> Result<()> {
        if score.schema_version != ScoreRow::schema_version() {
            return Err(anyhow!(
                "Invalid schema version: {} (expected {})",
                score.schema_version,
                ScoreRow::schema_version()
            ));
        }
        Ok(())
    }

    /// Clear cache
    pub fn clear(&mut self) {
        self.cache.clear();
    }
}

impl Default for ScoreLoader {
    fn default() -> Self {
        Self::new("data/research/scores", 300)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_check() {
        let loader = ScoreLoader::default();
        let score = ScoreRow {
            schema_version: "scores_v1".to_string(),
            condition_id: "test".to_string(),
            ts: 100,
            model_version: "v1".to_string(),
            score_yes_15s: 0.5,
            score_yes_30s: 0.5,
            score_yes_45s: 0.5,
            score_no_15s: 0.5,
            score_no_30s: 0.5,
            score_no_45s: 0.5,
            risk_yes_30s: 0.1,
            risk_no_30s: 0.1,
            fresh_until_ts: 200,
        };

        assert!(loader.is_fresh(&score, 150));
        assert!(!loader.is_fresh(&score, 250));
    }

    #[test]
    fn schema_validation() {
        let loader = ScoreLoader::default();
        let valid = ScoreRow {
            schema_version: "scores_v1".to_string(),
            condition_id: "test".to_string(),
            ts: 100,
            model_version: "v1".to_string(),
            score_yes_15s: 0.5,
            score_yes_30s: 0.5,
            score_yes_45s: 0.5,
            score_no_15s: 0.5,
            score_no_30s: 0.5,
            score_no_45s: 0.5,
            risk_yes_30s: 0.1,
            risk_no_30s: 0.1,
            fresh_until_ts: 200,
        };
        assert!(loader.validate_schema(&valid).is_ok());

        let invalid = ScoreRow {
            schema_version: "scores_v2".to_string(),
            ..valid.clone()
        };
        assert!(loader.validate_schema(&invalid).is_err());
    }
}
