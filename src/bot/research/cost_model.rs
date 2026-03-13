//! Cost Model
//!
//! Polymarket fee structure and edge calculation.

use serde::{Deserialize, Serialize};

/// Cost model configuration
#[derive(Debug, Clone, Deserialize)]
pub struct CostModel {
    /// Taker fee rate (buying at ask)
    pub taker_fee: f64,
    /// Maker fee rate (providing liquidity, often negative = rebate)
    pub maker_fee: f64,
    /// Slippage buffer for cost estimation
    pub slippage_buffer: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            taker_fee: 0.02,      // 2% taker fee
            maker_fee: -0.01,     // 1% maker rebate
            slippage_buffer: 0.005, // 0.5% slippage
        }
    }
}

impl CostModel {
    pub fn new(taker_fee: f64, maker_fee: f64, slippage_buffer: f64) -> Self {
        Self { taker_fee, maker_fee, slippage_buffer }
    }

    /// Calculate net edge after fees
    ///
    /// For a trade: enter at entry_price, exit at exit_price
    /// Returns net profit as fraction of entry
    pub fn net_edge(&self, entry_price: f64, exit_price: f64, is_taker_entry: bool) -> f64 {
        let entry_fee = if is_taker_entry { self.taker_fee } else { self.maker_fee.abs() };
        let exit_fee = self.taker_fee; // Usually taker on exit

        let gross_edge = exit_price - entry_price;
        let total_fees = entry_price * entry_fee + exit_price * exit_fee;
        let slippage = entry_price * self.slippage_buffer;

        gross_edge - total_fees - slippage
    }

    /// Check if trade is profitable after fees
    pub fn is_profitable_after_fees(&self, entry_price: f64, exit_price: f64) -> bool {
        self.net_edge(entry_price, exit_price, true) > 0.0
    }

    /// Calculate break-even exit price
    pub fn break_even_price(&self, entry_price: f64, is_taker_entry: bool) -> f64 {
        let entry_fee = if is_taker_entry { self.taker_fee } else { self.maker_fee.abs() };
        let total_cost_rate = entry_fee + self.taker_fee + self.slippage_buffer;
        entry_price * (1.0 + total_cost_rate)
    }

    /// Calculate fee for a given size
    pub fn calculate_fee(&self, size_usd: f64, is_taker: bool) -> f64 {
        let rate = if is_taker { self.taker_fee } else { self.maker_fee };
        size_usd * rate
    }

    /// Calculate total round-trip cost
    pub fn round_trip_cost(&self, size_usd: f64) -> f64 {
        let entry_fee = size_usd * self.taker_fee;
        let exit_fee = size_usd * self.taker_fee;
        let slippage = size_usd * self.slippage_buffer * 2.0;
        entry_fee + exit_fee + slippage
    }
}

/// Trade cost breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub entry_fee: f64,
    pub exit_fee: f64,
    pub slippage: f64,
    pub total_cost: f64,
    pub net_edge: f64,
}

impl CostModel {
    /// Get detailed cost breakdown
    pub fn breakdown(&self, entry_price: f64, exit_price: f64, size_usd: f64) -> CostBreakdown {
        let entry_fee = size_usd * self.taker_fee;
        let exit_fee = size_usd * self.taker_fee;
        let slippage = size_usd * self.slippage_buffer * 2.0;
        let total_cost = entry_fee + exit_fee + slippage;

        let gross_pnl = (exit_price - entry_price) * size_usd / entry_price;
        let net_edge = gross_pnl - total_cost;

        CostBreakdown {
            entry_fee,
            exit_fee,
            slippage,
            total_cost,
            net_edge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_edge_calculation() {
        let model = CostModel::default();
        // Enter at 0.50, exit at 0.55
        let edge = model.net_edge(0.50, 0.55, true);
        // Gross edge = 0.05
        // Entry fee = 0.50 * 0.02 = 0.01
        // Exit fee = 0.55 * 0.02 = 0.011
        // Slippage = 0.50 * 0.005 = 0.0025
        // Net = 0.05 - 0.01 - 0.011 - 0.0025 = 0.0265
        assert!(edge > 0.02 && edge < 0.03);
    }

    #[test]
    fn not_profitable_with_small_move() {
        let model = CostModel::default();
        // Enter at 0.50, exit at 0.51 (2% move, but fees eat it)
        assert!(!model.is_profitable_after_fees(0.50, 0.51));
    }

    #[test]
    fn break_even_price() {
        let model = CostModel::default();
        let entry = 0.50;
        let break_even = model.break_even_price(entry, true);
        // Should be higher than entry
        assert!(break_even > entry);
        // At break-even, net edge should be ~0
        let edge = model.net_edge(entry, break_even, true);
        assert!(edge.abs() < 0.001);
    }

    #[test]
    fn round_trip_cost() {
        let model = CostModel::default();
        let cost = model.round_trip_cost(100.0);
        // 2% + 2% + 0.5% * 2 = 5%
        assert!(cost > 4.0 && cost < 6.0);
    }
}
