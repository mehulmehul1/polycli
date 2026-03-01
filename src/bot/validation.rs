use serde::Serialize;
use std::fs::File;
use std::io::Write;

#[derive(Serialize, Clone, Default)]
pub struct TradeRecord {
    pub market_slug: String,
    pub token_side: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl_percent: f64,
    pub pnl_usd: f64,
    pub bankroll_after: f64,
    pub duration_seconds: i64,
}

#[derive(Serialize, Clone, Default)]
pub struct MarketRecord {
    pub market_slug: String,
    pub trades: usize,
    pub total_pnl_percent: f64,
    pub wins: usize,
    pub losses: usize,
}

#[derive(Default)]
pub struct ValidationTracker {
    pub trades: Vec<TradeRecord>,
    pub markets: Vec<MarketRecord>,
    pub current_market_trades: Vec<TradeRecord>,
    pub completed_markets: usize,
    pub max_markets: usize,
    pub starting_capital: f64,
    pub session_id: String,

    // Participation metrics
    pub signals_generated: usize,
    pub entries_taken: usize,
    pub entries_blocked_by_filter: usize,
}

impl ValidationTracker {
    pub fn new(max_markets: usize) -> Self {
        let session_id = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

        // Ensure validation directory exists
        let _ = std::fs::create_dir_all("validation");

        Self {
            max_markets,
            starting_capital: 4.0,
            session_id,
            ..Default::default()
        }
    }

    /// Record a signal was generated (bias + acceleration)
    pub fn record_signal(&mut self) {
        self.signals_generated += 1;
    }

    /// Record an entry was taken
    pub fn record_entry_taken(&mut self) {
        self.entries_taken += 1;
    }

    /// Record an entry was blocked by the structural filter
    pub fn record_entry_blocked(&mut self) {
        self.entries_blocked_by_filter += 1;
    }

    /// Calculate participation rate: entries_taken / signals_generated
    pub fn participation_rate(&self) -> f64 {
        if self.signals_generated == 0 {
            return 0.0;
        }
        (self.entries_taken as f64 / self.signals_generated as f64) * 100.0
    }

    pub fn record_trade(
        &mut self,
        market_slug: String,
        token_side: String,
        entry_price: f64,
        exit_price: f64,
        pnl_percent: f64,
        duration_seconds: i64,
        pnl_usd: f64,
        bankroll_after: f64,
    ) {
        let record = TradeRecord {
            market_slug,
            token_side,
            entry_price,
            exit_price,
            pnl_percent,
            pnl_usd,
            bankroll_after,
            duration_seconds,
        };
        self.current_market_trades.push(record.clone());
        self.trades.push(record);
        let _ = self.export_csv();
    }

    pub fn finalize_market(&mut self, market_slug: String, realized_pnl: f64) {
        let mut wins = 0;
        let mut losses = 0;

        for trade in &self.current_market_trades {
            if trade.pnl_percent > 0.0 {
                wins += 1;
            } else if trade.pnl_percent < 0.0 {
                losses += 1;
            }
        }

        self.markets.push(MarketRecord {
            market_slug,
            trades: self.current_market_trades.len(),
            total_pnl_percent: realized_pnl,
            wins,
            losses,
        });

        self.current_market_trades.clear();
        self.completed_markets += 1;
        let _ = self.export_json();
    }

    pub fn export_json(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = format!("validation/session_{}_summary.json", self.session_id);
        let file = File::create(path)?;

        let total_trades = self.trades.len();
        let wins: Vec<f64> = self
            .trades
            .iter()
            .map(|t| t.pnl_percent)
            .filter(|&p| p > 0.0)
            .collect();
        let losses: Vec<f64> = self
            .trades
            .iter()
            .map(|t| t.pnl_percent)
            .filter(|&p| p < 0.0)
            .collect();

        let win_rate = if total_trades > 0 {
            (wins.len() as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };
        let avg_win = if !wins.is_empty() {
            wins.iter().sum::<f64>() / wins.len() as f64
        } else {
            0.0
        };
        let avg_loss = if !losses.is_empty() {
            losses.iter().sum::<f64>() / losses.len() as f64
        } else {
            0.0
        };
        let total_pnl: f64 = self.trades.iter().map(|t| t.pnl_percent).sum();
        let max_win = wins.iter().cloned().fold(0.0, f64::max);
        let max_loss = losses.iter().cloned().fold(0.0, f64::min);

        let net_usd: f64 = self.trades.iter().map(|t| t.pnl_usd).sum();
        let ending_capital = self.starting_capital + net_usd;

        #[derive(Serialize)]
        struct ExportData {
            markets: Vec<MarketRecord>,
            total_trades: usize,
            completed_markets: usize,
            win_rate_pct: f64,
            avg_win_pct: f64,
            avg_loss_pct: f64,
            total_pnl_pct: f64,
            max_win_pct: f64,
            max_loss_pct: f64,
            starting_capital_usd: f64,
            ending_capital_usd: f64,
            net_profit_usd: f64,
            return_on_capital_pct: f64,
            // Participation metrics
            signals_generated: usize,
            entries_taken: usize,
            entries_blocked_by_filter: usize,
            participation_rate_pct: f64,
        }
        let data = ExportData {
            markets: self.markets.clone(),
            total_trades,
            completed_markets: self.completed_markets,
            win_rate_pct: win_rate,
            avg_win_pct: avg_win * 100.0,
            avg_loss_pct: avg_loss * 100.0,
            total_pnl_pct: total_pnl * 100.0,
            max_win_pct: max_win * 100.0,
            max_loss_pct: max_loss * 100.0,
            starting_capital_usd: self.starting_capital,
            ending_capital_usd: ending_capital,
            net_profit_usd: net_usd,
            return_on_capital_pct: (net_usd / self.starting_capital) * 100.0,
            signals_generated: self.signals_generated,
            entries_taken: self.entries_taken,
            entries_blocked_by_filter: self.entries_blocked_by_filter,
            participation_rate_pct: self.participation_rate(),
        };
        serde_json::to_writer_pretty(file, &data)?;
        Ok(())
    }

    pub fn export_csv(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = format!("validation/session_{}_trades.csv", self.session_id);
        let file_exists = std::path::Path::new(&path).exists();

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        if !file_exists {
            writeln!(file, "market_slug,token_side,entry_price,exit_price,pnl_percent,pnl_usd,bankroll_after,duration_seconds")?;
        }

        if let Some(t) = self.trades.last() {
            writeln!(
                file,
                "{},{},{:.4},{:.4},{:.2},{:.4},{:.2},{}",
                t.market_slug,
                t.token_side,
                t.entry_price,
                t.exit_price,
                t.pnl_percent * 100.0,
                t.pnl_usd,
                t.bankroll_after,
                t.duration_seconds
            )?;
        }
        Ok(())
    }

    pub fn print_summary(&self) {
        let total_trades = self.trades.len();
        let wins: Vec<f64> = self
            .trades
            .iter()
            .map(|t| t.pnl_percent)
            .filter(|&p| p > 0.0)
            .collect();
        let losses: Vec<f64> = self
            .trades
            .iter()
            .map(|t| t.pnl_percent)
            .filter(|&p| p < 0.0)
            .collect();

        let win_rate = if total_trades > 0 {
            (wins.len() as f64 / total_trades as f64) * 100.0
        } else {
            0.0
        };
        let avg_win = if !wins.is_empty() {
            wins.iter().sum::<f64>() / wins.len() as f64
        } else {
            0.0
        };
        let avg_loss = if !losses.is_empty() {
            losses.iter().sum::<f64>() / losses.len() as f64
        } else {
            0.0
        };
        let total_pnl: f64 = self.trades.iter().map(|t| t.pnl_percent).sum();

        let max_win = wins.iter().cloned().fold(0.0, f64::max);
        let max_loss = losses.iter().cloned().fold(0.0, f64::min);

        let net_usd: f64 = self.trades.iter().map(|t| t.pnl_usd).sum();
        let ending_capital = self.starting_capital + net_usd;

        println!("\n============ FINAL PERFORMANCE ============");
        println!("Starting Capital: ${:.2}", self.starting_capital);
        println!("Ending Capital: ${:.2}", ending_capital);
        println!("Net USD: {:+.4}", net_usd);
        println!(
            "Capital Return: {:.2}%",
            (net_usd / self.starting_capital) * 100.0
        );
        println!("--------------------------------------------");
        println!("Markets: {}", self.completed_markets);
        println!("Total Trades: {}", total_trades);
        println!("Win Rate: {:.2}%", win_rate);
        println!("Average Win: {:.4}%", avg_win * 100.0);
        println!("Average Loss: {:.4}%", avg_loss * 100.0);
        println!("Total PnL (Strategy Edge): {:.4}%", total_pnl * 100.0);
        println!("Max Win: {:.4}%", max_win * 100.0);
        println!("Max Loss: {:.4}%", max_loss * 100.0);
        println!("--------------------------------------------");
        println!("=== PARTICIPATION METRICS ===");
        println!("Signals Generated: {}", self.signals_generated);
        println!("Entries Taken: {}", self.entries_taken);
        println!(
            "Entries Blocked by Filter: {}",
            self.entries_blocked_by_filter
        );
        println!("Participation Rate: {:.2}%", self.participation_rate());
        println!("--------------------------------------------");
        if self.participation_rate() < 40.0 {
            println!("WARNING: Participation rate < 40% - model may be over-filtered");
        } else if self.participation_rate() > 80.0 {
            println!("NOTE: Participation rate > 80% - model may be over-permissive");
        } else {
            println!("Participation rate in healthy range (40-80%)");
        }
        println!("============================================");
        println!("Session ID: {}", self.session_id);
        println!("Results saved to: validation/");
    }
}
