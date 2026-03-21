//! Fair Value Monitor
//!
//! Tracks fair value predictions and outcomes for model validation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single fair value prediction with optional outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairValuePrediction {
    /// Timestamp of prediction
    pub ts: i64,
    /// Market condition ID
    pub condition_id: Option<String>,
    /// Calibrated fair probability
    pub fair_prob_calibrated: f64,
    /// Market YES ask price at prediction time
    pub market_yes_ask: f64,
    /// Predicted edge (fair_prob - market_ask)
    pub edge_predicted: f64,
    /// Direction taken (Yes/No)
    pub direction: Option<String>,
    /// Actual outcome (true = YES won, false = NO won)
    /// None if market not yet resolved
    pub outcome: Option<bool>,
    /// Market slug for reference
    pub market_slug: Option<String>,
}

impl FairValuePrediction {
    /// Create a new prediction
    pub fn new(
        ts: i64,
        condition_id: Option<String>,
        fair_prob_calibrated: f64,
        market_yes_ask: f64,
        edge_predicted: f64,
    ) -> Self {
        Self {
            ts,
            condition_id,
            fair_prob_calibrated,
            market_yes_ask,
            edge_predicted,
            direction: None,
            outcome: None,
            market_slug: None,
        }
    }

    /// Set the outcome of this prediction
    pub fn with_outcome(mut self, outcome: bool) -> Self {
        self.outcome = Some(outcome);
        self
    }

    /// Set the direction taken
    pub fn with_direction(mut self, direction: String) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Set the market slug
    pub fn with_market_slug(mut self, slug: String) -> Self {
        self.market_slug = Some(slug);
        self
    }

    /// Calculate squared error for this prediction (if resolved)
    pub fn squared_error(&self) -> Option<f64> {
        self.outcome.map(|o| {
            let target = if o { 1.0 } else { 0.0 };
            (self.fair_prob_calibrated - target).powi(2)
        })
    }

    /// Check if prediction was correct
    pub fn was_correct(&self) -> Option<bool> {
        self.outcome.map(|o| {
            // Prediction is correct if:
            // - fair_prob > 0.5 and outcome was YES
            // - fair_prob < 0.5 and outcome was NO
            // For exactly 0.5, consider it incorrect (no edge)
            if self.fair_prob_calibrated > 0.5 {
                o
            } else if self.fair_prob_calibrated < 0.5 {
                !o
            } else {
                false
            }
        })
    }
}

/// Monitor for tracking fair value model performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairValueMonitor {
    /// All predictions tracked
    predictions: Vec<FairValuePrediction>,
    /// Indexed by condition_id for quick lookup
    index: HashMap<String, usize>,
    /// Session start time
    session_start: i64,
}

impl Default for FairValueMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl FairValueMonitor {
    /// Create a new monitor
    pub fn new() -> Self {
        Self {
            predictions: Vec::new(),
            index: HashMap::new(),
            session_start: chrono::Utc::now().timestamp(),
        }
    }

    /// Record a prediction
    pub fn record(&mut self, prediction: FairValuePrediction) {
        // Only record if we have a condition_id
        let Some(ref condition_id) = prediction.condition_id else {
            return;
        };

        // Check if we already have a prediction for this market
        if let Some(&idx) = self.index.get(condition_id) {
            // Update existing prediction
            self.predictions[idx] = prediction;
        } else {
            // Add new prediction
            let idx = self.predictions.len();
            self.index.insert(condition_id.clone(), idx);
            self.predictions.push(prediction);
        }
    }

    /// Record an outcome for an existing prediction
    pub fn record_outcome(&mut self, condition_id: &str, outcome: bool) {
        if let Some(&idx) = self.index.get(condition_id) {
            self.predictions[idx].outcome = Some(outcome);
        }
    }

    /// Calculate Brier score for resolved predictions
    ///
    /// Brier score = mean((predicted_probability - actual_outcome)^2)
    /// Lower is better (0 = perfect, 0.25 = random, 1 = always wrong)
    pub fn brier_score(&self) -> Option<f64> {
        let resolved: Vec<_> = self
            .predictions
            .iter()
            .filter_map(|p| p.outcome.map(|o| (p.fair_prob_calibrated, o)))
            .collect();

        if resolved.is_empty() {
            return None;
        }

        let sum_squared_error: f64 = resolved
            .iter()
            .map(|(prob, outcome)| {
                let target = if *outcome { 1.0 } else { 0.0 };
                (prob - target).powi(2)
            })
            .sum();

        Some(sum_squared_error / resolved.len() as f64)
    }

    /// Calculate overall win rate
    pub fn win_rate(&self) -> Option<f64> {
        let resolved: Vec<_> = self
            .predictions
            .iter()
            .filter_map(|p| p.was_correct())
            .collect();

        if resolved.is_empty() {
            return None;
        }

        let wins = resolved.iter().filter(|&&correct| correct).count();
        Some(wins as f64 / resolved.len() as f64)
    }

    /// Calculate edge correlation with outcomes
    ///
    /// Returns correlation between predicted edge and actual outcome
    pub fn edge_correlation(&self) -> Option<f64> {
        let resolved: Vec<_> = self
            .predictions
            .iter()
            .filter_map(|p| {
                p.outcome.map(|o| {
                    let outcome_val = if o { 1.0 } else { 0.0 };
                    (p.edge_predicted, outcome_val)
                })
            })
            .collect();

        if resolved.len() < 10 {
            return None;
        }

        // Calculate Pearson correlation
        let n = resolved.len() as f64;
        let mean_edge: f64 = resolved.iter().map(|(e, _)| e).sum::<f64>() / n;
        let mean_outcome: f64 = resolved.iter().map(|(_, o)| o).sum::<f64>() / n;

        let numerator: f64 = resolved
            .iter()
            .map(|(e, o)| (e - mean_edge) * (o - mean_outcome))
            .sum();

        let var_edge: f64 = resolved.iter().map(|(e, _)| (e - mean_edge).powi(2)).sum();
        let var_outcome: f64 = resolved
            .iter()
            .map(|(_, o)| (o - mean_outcome).powi(2))
            .sum();

        let denominator = var_edge.sqrt() * var_outcome.sqrt();

        if denominator > 1e-10 {
            Some(numerator / denominator)
        } else {
            None
        }
    }

    /// Get win rate by probability bucket
    ///
    /// Returns win rates for different probability ranges
    pub fn win_rate_by_bucket(&self) -> HashMap<String, (usize, usize)> {
        let mut buckets: HashMap<String, (usize, usize)> = HashMap::new();

        for prediction in &self.predictions {
            if let Some(correct) = prediction.was_correct() {
                let bucket = if prediction.fair_prob_calibrated < 0.4 {
                    "low (<0.4)"
                } else if prediction.fair_prob_calibrated < 0.6 {
                    "mid (0.4-0.6)"
                } else {
                    "high (>0.6)"
                };

                let entry = buckets.entry(bucket.to_string()).or_insert((0, 0));
                entry.0 += 1; // total
                if correct {
                    entry.1 += 1; // wins
                }
            }
        }

        buckets
    }

    /// Get statistics summary
    pub fn summary(&self) -> MonitorSummary {
        let total = self.predictions.len();
        let resolved = self.predictions.iter().filter(|p| p.outcome.is_some()).count();

        let avg_edge = if total > 0 {
            let sum: f64 = self.predictions.iter().map(|p| p.edge_predicted.abs()).sum();
            Some(sum / total as f64)
        } else {
            None
        };

        MonitorSummary {
            session_start: self.session_start,
            total_predictions: total,
            resolved_predictions: resolved,
            unresolved_predictions: total - resolved,
            brier_score: self.brier_score(),
            win_rate: self.win_rate(),
            edge_correlation: self.edge_correlation(),
            avg_edge_absolute: avg_edge,
            win_rate_by_bucket: self.win_rate_by_bucket(),
        }
    }

    /// Get all predictions
    pub fn predictions(&self) -> &[FairValuePrediction] {
        &self.predictions
    }

    /// Get predictions for a specific market
    pub fn get_prediction(&self, condition_id: &str) -> Option<&FairValuePrediction> {
        self.index
            .get(condition_id)
            .and_then(|&idx| self.predictions.get(idx))
    }

    /// Clear all predictions
    pub fn clear(&mut self) {
        self.predictions.clear();
        self.index.clear();
    }

    /// Export predictions as JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.predictions)
    }

    /// Import predictions from JSON
    pub fn from_json(&mut self, json: &str) -> Result<(), serde_json::Error> {
        let predictions: Vec<FairValuePrediction> = serde_json::from_str(json)?;
        self.clear();
        for pred in predictions {
            self.record(pred);
        }
        Ok(())
    }

    /// Get session duration in seconds
    pub fn session_duration(&self) -> i64 {
        chrono::Utc::now().timestamp() - self.session_start
    }
}

/// Summary statistics for the fair value monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorSummary {
    pub session_start: i64,
    pub total_predictions: usize,
    pub resolved_predictions: usize,
    pub unresolved_predictions: usize,
    pub brier_score: Option<f64>,
    pub win_rate: Option<f64>,
    pub edge_correlation: Option<f64>,
    pub avg_edge_absolute: Option<f64>,
    pub win_rate_by_bucket: HashMap<String, (usize, usize)>, // (total, wins)
}

impl std::fmt::Display for MonitorSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Fair Value Monitor Summary ===")?;
        writeln!(f, "Session start: {}", self.session_start)?;
        writeln!(f, "Total predictions: {}", self.total_predictions)?;
        writeln!(f, "Resolved: {} / Unresolved: {}",
            self.resolved_predictions, self.unresolved_predictions)?;

        if let Some(br) = self.brier_score {
            writeln!(f, "Brier score: {:.4} (lower is better)", br)?;
        }
        if let Some(wr) = self.win_rate {
            writeln!(f, "Win rate: {:.2}%", wr * 100.0)?;
        }
        if let Some(ec) = self.edge_correlation {
            writeln!(f, "Edge correlation: {:.4}", ec)?;
        }
        if let Some(ae) = self.avg_edge_absolute {
            writeln!(f, "Avg edge: {:.4}", ae)?;
        }

        writeln!(f, "\nWin rate by bucket:")?;
        for (bucket, (total, wins)) in &self.win_rate_by_bucket {
            let rate = (*wins as f64 / *total as f64) * 100.0;
            writeln!(f, "  {}: {}/{} ({:.1}%)", bucket, wins, total, rate)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_records_prediction() {
        let mut monitor = FairValueMonitor::new();
        let pred = FairValuePrediction::new(1000, Some("test-1".to_string()), 0.55, 0.50, 0.05);

        monitor.record(pred);

        assert_eq!(monitor.predictions().len(), 1);
        assert_eq!(monitor.predictions()[0].condition_id, Some("test-1".to_string()));
    }

    #[test]
    fn monitor_updates_existing_prediction() {
        let mut monitor = FairValueMonitor::new();
        let pred1 = FairValuePrediction::new(1000, Some("test-1".to_string()), 0.55, 0.50, 0.05);
        let pred2 = FairValuePrediction::new(1001, Some("test-1".to_string()), 0.60, 0.50, 0.10);

        monitor.record(pred1);
        monitor.record(pred2);

        assert_eq!(monitor.predictions().len(), 1);
        assert_eq!(monitor.predictions()[0].edge_predicted, 0.10);
    }

    #[test]
    fn brier_score_calculation() {
        let mut monitor = FairValueMonitor::new();

        // Perfect predictions (Brier = 0)
        monitor.record(FairValuePrediction::new(1000, Some("test-1".to_string()), 0.9, 0.5, 0.4).with_outcome(true));
        monitor.record(FairValuePrediction::new(1001, Some("test-2".to_string()), 0.1, 0.5, -0.4).with_outcome(false));

        let brier = monitor.brier_score();
        assert!(brier.is_some());
        assert!(brier.unwrap() < 0.1); // Should be low for good predictions
    }

    #[test]
    fn win_rate_calculation() {
        let mut monitor = FairValueMonitor::new();

        monitor.record(FairValuePrediction::new(1000, Some("test-1".to_string()), 0.7, 0.5, 0.2).with_outcome(true));
        monitor.record(FairValuePrediction::new(1001, Some("test-2".to_string()), 0.7, 0.5, 0.2).with_outcome(false));
        monitor.record(FairValuePrediction::new(1002, Some("test-3".to_string()), 0.3, 0.5, -0.2).with_outcome(false));

        let win_rate = monitor.win_rate();
        assert_eq!(win_rate, Some(2.0 / 3.0));
    }

    #[test]
    fn win_rate_by_bucket() {
        let mut monitor = FairValueMonitor::new();

        monitor.record(FairValuePrediction::new(1000, Some("test-1".to_string()), 0.8, 0.5, 0.3).with_outcome(true));
        monitor.record(FairValuePrediction::new(1001, Some("test-2".to_string()), 0.3, 0.5, -0.2).with_outcome(false));

        let buckets = monitor.win_rate_by_bucket();
        assert!(buckets.contains_key("high (>0.6)"));
        assert!(buckets.contains_key("low (<0.4)"));
    }

    #[test]
    fn summary_displays_correctly() {
        let monitor = FairValueMonitor::new();
        let summary = monitor.summary();

        assert_eq!(summary.total_predictions, 0);
        assert_eq!(summary.resolved_predictions, 0);
    }

    #[test]
    fn squared_error_for_prediction() {
        let pred = FairValuePrediction::new(1000, Some("test-1".to_string()), 0.7, 0.5, 0.2).with_outcome(true);
        assert_eq!(pred.squared_error(), Some((0.7_f64 - 1.0).powi(2)));

        let pred_no_outcome = FairValuePrediction::new(1000, Some("test-1".to_string()), 0.7, 0.5, 0.2);
        assert!(pred_no_outcome.squared_error().is_none());
    }

    #[test]
    fn was_correct_check() {
        let pred_correct = FairValuePrediction::new(1000, Some("test-1".to_string()), 0.7, 0.5, 0.2).with_outcome(true);
        assert_eq!(pred_correct.was_correct(), Some(true));

        let pred_incorrect = FairValuePrediction::new(1000, Some("test-1".to_string()), 0.7, 0.5, 0.2).with_outcome(false);
        assert_eq!(pred_incorrect.was_correct(), Some(false));
    }
}
