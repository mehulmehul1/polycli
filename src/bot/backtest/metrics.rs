use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResult {
    pub market_slug: String,
    pub side: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl_percent: f64,
    pub pnl_usd: f64,
    pub duration_seconds: i64,
    pub entry_timestamp: i64,
    pub exit_timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BacktestMetrics {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub avg_win_pct: f64,
    pub avg_loss_pct: f64,
    pub total_pnl_pct: f64,
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,
    pub expectancy: f64,
    pub profit_factor: f64,
    pub max_consecutive_wins: usize,
    pub max_consecutive_losses: usize,
    pub starting_capital: f64,
    pub ending_capital: f64,
    pub return_on_capital_pct: f64,
    pub trades: Vec<TradeResult>,
    pub equity_curve: Vec<f64>,
}

impl BacktestMetrics {
    pub fn new() -> Self {
        Self {
            starting_capital: 100.0,
            ..Default::default()
        }
    }

    pub fn from_trades(trades: Vec<TradeResult>, starting_capital: f64) -> Self {
        let mut metrics = Self::new();
        metrics.starting_capital = starting_capital;
        metrics.trades = trades;
        metrics.calculate();
        metrics
    }

    pub fn calculate(&mut self) {
        if self.trades.is_empty() {
            return;
        }

        self.total_trades = self.trades.len();

        let wins: Vec<&TradeResult> = self.trades.iter().filter(|t| t.pnl_percent > 0.0).collect();
        let losses: Vec<&TradeResult> =
            self.trades.iter().filter(|t| t.pnl_percent < 0.0).collect();

        self.winning_trades = wins.len();
        self.losing_trades = losses.len();

        self.win_rate = (self.winning_trades as f64 / self.total_trades as f64) * 100.0;

        self.avg_win_pct = if !wins.is_empty() {
            wins.iter().map(|t| t.pnl_percent).sum::<f64>() / wins.len() as f64
        } else {
            0.0
        };

        self.avg_loss_pct = if !losses.is_empty() {
            losses.iter().map(|t| t.pnl_percent).sum::<f64>() / losses.len() as f64
        } else {
            0.0
        };

        self.total_pnl_pct = self.trades.iter().map(|t| t.pnl_percent).sum();

        let net_usd: f64 = self.trades.iter().map(|t| t.pnl_usd).sum();
        self.ending_capital = self.starting_capital + net_usd;
        self.return_on_capital_pct = (net_usd / self.starting_capital) * 100.0;

        let gross_profit: f64 = wins.iter().map(|t| t.pnl_percent.abs()).sum();
        let gross_loss: f64 = losses.iter().map(|t| t.pnl_percent.abs()).sum();
        self.profit_factor = if gross_loss > 0.0 {
            gross_profit / gross_loss
        } else {
            f64::INFINITY
        };

        self.expectancy = if self.total_trades > 0 {
            self.total_pnl_pct / self.total_trades as f64
        } else {
            0.0
        };

        self.calculate_drawdown();
        self.calculate_sharpe();
        self.calculate_consecutive();
        self.build_equity_curve();
    }

    fn calculate_drawdown(&mut self) {
        let mut equity = self.starting_capital;
        let mut peak = equity;
        let mut max_dd = 0.0;

        for trade in &self.trades {
            equity += trade.pnl_usd;
            if equity > peak {
                peak = equity;
            }
            let dd = (peak - equity) / peak * 100.0;
            if dd > max_dd {
                max_dd = dd;
            }
        }

        self.max_drawdown_pct = max_dd;
    }

    fn calculate_sharpe(&mut self) {
        if self.trades.len() < 2 {
            self.sharpe_ratio = 0.0;
            return;
        }

        let returns: Vec<f64> = self.trades.iter().map(|t| t.pnl_percent).collect();
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;

        let variance =
            returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;

        let std_dev = variance.sqrt();

        self.sharpe_ratio = if std_dev > 0.0 {
            (mean / std_dev) * (252.0_f64).sqrt()
        } else {
            0.0
        };
    }

    fn calculate_consecutive(&mut self) {
        let mut current_wins = 0;
        let mut current_losses = 0;
        let mut max_wins = 0;
        let mut max_losses = 0;

        for trade in &self.trades {
            if trade.pnl_percent > 0.0 {
                current_wins += 1;
                current_losses = 0;
                max_wins = max_wins.max(current_wins);
            } else if trade.pnl_percent < 0.0 {
                current_losses += 1;
                current_wins = 0;
                max_losses = max_losses.max(current_losses);
            }
        }

        self.max_consecutive_wins = max_wins;
        self.max_consecutive_losses = max_losses;
    }

    fn build_equity_curve(&mut self) {
        let mut equity = self.starting_capital;
        self.equity_curve = vec![equity];

        for trade in &self.trades {
            equity += trade.pnl_usd;
            self.equity_curve.push(equity);
        }
    }

    pub fn print_summary(&self) {
        println!("\n============ BACKTEST RESULTS ============");
        println!("Starting Capital: ${:.2}", self.starting_capital);
        println!("Ending Capital: ${:.2}", self.ending_capital);
        println!("Return: {:.2}%", self.return_on_capital_pct);
        println!("--------------------------------------------");
        println!("Total Trades: {}", self.total_trades);
        println!("Win Rate: {:.2}%", self.win_rate);
        println!(
            "Winning: {} | Losing: {}",
            self.winning_trades, self.losing_trades
        );
        println!("--------------------------------------------");
        println!("Avg Win: {:.4}%", self.avg_win_pct * 100.0);
        println!("Avg Loss: {:.4}%", self.avg_loss_pct * 100.0);
        println!("Total PnL: {:.4}%", self.total_pnl_pct * 100.0);
        println!("Expectancy: {:.6}", self.expectancy);
        println!("Profit Factor: {:.2}", self.profit_factor);
        println!("--------------------------------------------");
        println!("Max Drawdown: {:.2}%", self.max_drawdown_pct);
        println!("Sharpe Ratio: {:.2}", self.sharpe_ratio);
        println!("Max Consecutive Wins: {}", self.max_consecutive_wins);
        println!("Max Consecutive Losses: {}", self.max_consecutive_losses);
        println!("============================================");
    }

    pub fn export_json(&self, path: &str) -> anyhow::Result<()> {
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }

    pub fn export_csv(&self, path: &str) -> anyhow::Result<()> {
        let mut file = File::create(path)?;
        writeln!(
            file,
            "market_slug,side,entry_price,exit_price,pnl_percent,pnl_usd,duration_seconds"
        )?;

        for t in &self.trades {
            writeln!(
                file,
                "{},{},{:.4},{:.4},{:.4},{:.4},{}",
                t.market_slug,
                t.side,
                t.entry_price,
                t.exit_price,
                t.pnl_percent * 100.0,
                t.pnl_usd,
                t.duration_seconds
            )?;
        }

        Ok(())
    }

    pub fn export_equity_curve(&self, path: &str) -> anyhow::Result<()> {
        let mut file = File::create(path)?;
        writeln!(file, "trade_num,equity")?;

        for (i, &equity) in self.equity_curve.iter().enumerate() {
            writeln!(file, "{},{:.2}", i, equity)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSweepResult {
    pub entry_band_low: f64,
    pub entry_band_high: f64,
    pub metrics: BacktestMetrics,
}

impl ParameterSweepResult {
    pub fn print_comparison(results: &[Self]) {
        println!("\n======== PARAMETER SWEEP RESULTS ========");
        println!(
            "{:<15} {:<10} {:<10} {:<10} {:<12} {:<10}",
            "Band", "Win%", "Trades", "Return%", "Drawdown%", "Sharpe"
        );
        println!("{}", "-".repeat(70));

        for r in results {
            let band = format!("{:.2}-{:.2}", r.entry_band_low, r.entry_band_high);
            println!(
                "{:<15} {:<10.1} {:<10} {:<10.2} {:<12.2} {:<10.2}",
                band,
                r.metrics.win_rate,
                r.metrics.total_trades,
                r.metrics.return_on_capital_pct,
                r.metrics.max_drawdown_pct,
                r.metrics.sharpe_ratio
            );
        }
        println!("==========================================");
    }
}
