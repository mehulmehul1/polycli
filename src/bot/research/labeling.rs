//! Label Generation
//!
//! Compute labels for ML training from feature data.

use super::{CostModel, FeatureRow, LabelRow};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Label configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LabelConfig {
    /// Label horizons in seconds
    pub horizons: Vec<i32>,
    /// Fee buffer for edge calculation
    pub fee_buffer: f64,
    /// Slippage buffer
    pub slippage_buffer: f64,
}

impl Default for LabelConfig {
    fn default() -> Self {
        Self {
            horizons: vec![15, 30, 45],
            fee_buffer: 0.02,
            slippage_buffer: 0.005,
        }
    }
}

/// Label generator
pub struct Labeler {
    config: LabelConfig,
    cost_model: CostModel,
}

impl Labeler {
    pub fn new(config: LabelConfig) -> Self {
        Self {
            config,
            cost_model: CostModel::default(),
        }
    }

    /// Compute labels for a sequence of feature rows
    ///
    /// Rows must be sorted by (condition_id, ts)
    pub fn compute_labels(&self, rows: &[FeatureRow]) -> Vec<LabelRow> {
        if rows.is_empty() {
            return Vec::new();
        }

        let mut labels = Vec::with_capacity(rows.len());

        // Group by condition_id
        let mut start = 0;
        while start < rows.len() {
            let condition_id = &rows[start].condition_id;
            let mut end = start + 1;
            while end < rows.len() && rows[end].condition_id == *condition_id {
                end += 1;
            }

            // Compute labels for this market
            for i in start..end {
                let row = &rows[i];
                let label = self.compute_label_for_row(rows, i, end);
                labels.push(label);
            }

            start = end;
        }

        labels
    }

    fn compute_label_for_row(&self, rows: &[FeatureRow], idx: usize, end: usize) -> LabelRow {
        let row = &rows[idx];
        let ts = row.ts;
        let total_cost = self.config.fee_buffer + self.config.slippage_buffer;

        // Find future rows for each horizon
        let mut labels = LabelRow {
            condition_id: row.condition_id.clone(),
            ts,
            label_reachable_yes_15s: 0,
            label_reachable_yes_30s: 0,
            label_reachable_yes_45s: 0,
            label_reachable_no_15s: 0,
            label_reachable_no_30s: 0,
            label_reachable_no_45s: 0,
            label_edge_yes_15s: 0.0,
            label_edge_yes_30s: 0.0,
            label_edge_yes_45s: 0.0,
            label_edge_no_15s: 0.0,
            label_edge_no_30s: 0.0,
            label_edge_no_45s: 0.0,
            label_adverse_yes_30s: 0.0,
            label_adverse_no_30s: 0.0,
            label_resolution: None,
        };

        let entry_yes = row.yes_ask; // Enter YES by buying at ask
        let entry_no = row.no_ask;   // Enter NO by buying at ask

        for horizon in &self.config.horizons {
            let horizon_ts = ts + *horizon as i64;

            // Find future rows within horizon
            let future_rows: Vec<&FeatureRow> = rows[idx..end]
                .iter()
                .filter(|r| r.ts > ts && r.ts <= horizon_ts)
                .collect();

            if future_rows.is_empty() {
                continue;
            }

            // Compute reachable profit labels
            let (reachable_yes, edge_yes) = self.compute_reachable(&future_rows, entry_yes, total_cost, true);
            let (reachable_no, edge_no) = self.compute_reachable(&future_rows, entry_no, total_cost, false);

            // Compute adverse excursion
            let adverse_yes = self.compute_adverse(&future_rows, entry_yes, total_cost, true);
            let adverse_no = self.compute_adverse(&future_rows, entry_no, total_cost, false);

            match *horizon {
                15 => {
                    labels.label_reachable_yes_15s = reachable_yes;
                    labels.label_reachable_no_15s = reachable_no;
                    labels.label_edge_yes_15s = edge_yes;
                    labels.label_edge_no_15s = edge_no;
                }
                30 => {
                    labels.label_reachable_yes_30s = reachable_yes;
                    labels.label_reachable_no_30s = reachable_no;
                    labels.label_edge_yes_30s = edge_yes;
                    labels.label_edge_no_30s = edge_no;
                    labels.label_adverse_yes_30s = adverse_yes;
                    labels.label_adverse_no_30s = adverse_no;
                }
                45 => {
                    labels.label_reachable_yes_45s = reachable_yes;
                    labels.label_reachable_no_45s = reachable_no;
                    labels.label_edge_yes_45s = edge_yes;
                    labels.label_edge_no_45s = edge_no;
                }
                _ => {}
            }
        }

        labels
    }

    /// Compute reachable profit and max edge
    fn compute_reachable(
        &self,
        future_rows: &[&FeatureRow],
        entry_price: f64,
        total_cost: f64,
        is_yes: bool,
    ) -> (i8, f64) {
        let exit_prices: Vec<f64> = if is_yes {
            future_rows.iter().map(|r| r.yes_bid).collect() // Exit YES by selling at bid
        } else {
            future_rows.iter().map(|r| r.no_bid).collect() // Exit NO by selling at bid
        };

        let mut max_edge = f64::MIN;
        let mut reachable = 0i8;

        for exit_price in exit_prices {
            let edge = exit_price - entry_price - total_cost;
            max_edge = max_edge.max(edge);
            if edge > 0.0 {
                reachable = 1;
            }
        }

        (reachable, max_edge)
    }

    /// Compute adverse excursion (worst drawdown)
    fn compute_adverse(
        &self,
        future_rows: &[&FeatureRow],
        entry_price: f64,
        total_cost: f64,
        is_yes: bool,
    ) -> f64 {
        let exit_prices: Vec<f64> = if is_yes {
            future_rows.iter().map(|r| r.yes_bid).collect()
        } else {
            future_rows.iter().map(|r| r.no_bid).collect()
        };

        let mut worst_drawdown = 0.0f64;

        for exit_price in exit_prices {
            let pnl = exit_price - entry_price - total_cost;
            worst_drawdown = worst_drawdown.min(pnl);
        }

        worst_drawdown
    }
}

impl Default for Labeler {
    fn default() -> Self {
        Self::new(LabelConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(condition_id: &str, ts: i64, yes_bid: f64, yes_ask: f64) -> FeatureRow {
        FeatureRow {
            schema_version: "features_v1".to_string(),
            condition_id: condition_id.to_string(),
            market_slug: format!("{}-market", condition_id),
            instrument: condition_id.to_string(),
            asset: "btc".to_string(),
            duration: "5m".to_string(),
            market_family: "updown_open_close".to_string(),
            market_start_ts: ts - 100,
            market_end_ts: ts + 200,
            ts,
            yes_bid,
            yes_ask,
            no_bid: 1.0 - yes_ask,
            no_ask: 1.0 - yes_bid,
            yes_mid: (yes_bid + yes_ask) / 2.0,
            no_mid: 1.0 - (yes_bid + yes_ask) / 2.0,
            yes_spread: yes_ask - yes_bid,
            no_spread: (1.0 - yes_bid) - (1.0 - yes_ask),
            book_sum: yes_ask + (1.0 - yes_bid),
            book_gap: yes_ask + (1.0 - yes_bid) - 1.0,
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
        }
    }

    #[test]
    fn compute_labels_basic() {
        let labeler = Labeler::default();

        // Create a sequence of rows with price movement
        let rows = vec![
            make_row("test-1", 0, 0.48, 0.50),   // Entry at 0.50
            make_row("test-1", 10, 0.49, 0.51),
            make_row("test-1", 20, 0.52, 0.54),  // Exit at 0.52 - profitable
            make_row("test-1", 30, 0.53, 0.55),
            make_row("test-1", 40, 0.51, 0.53),
        ];

        let labels = labeler.compute_labels(&rows);
        assert_eq!(labels.len(), 5);

        // First row should have reachable label at 30s
        assert_eq!(labels[0].label_reachable_yes_30s, 1);
        assert!(labels[0].label_edge_yes_30s > 0.0);
    }

    #[test]
    fn no_label_for_no_movement() {
        let labeler = Labeler::default();

        // Flat prices - no edge after fees
        let rows = vec![
            make_row("test-2", 0, 0.49, 0.51),
            make_row("test-2", 10, 0.49, 0.51),
            make_row("test-2", 20, 0.49, 0.51),
            make_row("test-2", 30, 0.49, 0.51),
        ];

        let labels = labeler.compute_labels(&rows);
        // Should not be reachable (spread + fees eat any edge)
        assert_eq!(labels[0].label_reachable_yes_30s, 0);
    }
}
