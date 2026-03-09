use crate::bot::backtest::metrics::{BacktestMetrics, TradeResult};
use rand::prelude::*;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};
use statrs::statistics::Statistics;
use std::fs::File;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    pub num_simulations: usize,
    pub num_trades_per_sim: usize,
    pub starting_capital: f64,
    pub position_size_pct: f64,
    pub win_rate: f64,
    pub avg_win_pct: f64,
    pub avg_loss_pct: f64,
    pub win_std_dev: f64,
    pub loss_std_dev: f64,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            num_simulations: 10_000,
            num_trades_per_sim: 500,
            starting_capital: 100.0,
            position_size_pct: 0.02,
            win_rate: 0.55,
            avg_win_pct: 0.03,
            avg_loss_pct: -0.02,
            win_std_dev: 0.02,
            loss_std_dev: 0.015,
        }
    }
}

impl SimulationConfig {
    pub fn from_metrics(metrics: &BacktestMetrics) -> Self {
        let wins: Vec<f64> = metrics
            .trades
            .iter()
            .filter(|t| t.pnl_percent > 0.0)
            .map(|t| t.pnl_percent)
            .collect();

        let losses: Vec<f64> = metrics
            .trades
            .iter()
            .filter(|t| t.pnl_percent < 0.0)
            .map(|t| t.pnl_percent)
            .collect();

        let win_std = if wins.len() > 1 {
            let data: Vec<f64> = wins.clone();
            let mean = Statistics::mean(&data);
            (data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (data.len() - 1) as f64).sqrt()
        } else {
            0.02
        };

        let loss_std = if losses.len() > 1 {
            let data: Vec<f64> = losses.clone();
            let mean = Statistics::mean(&data);
            (data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (data.len() - 1) as f64).sqrt()
        } else {
            0.015
        };

        Self {
            num_simulations: 10_000,
            num_trades_per_sim: metrics.total_trades.max(100),
            starting_capital: metrics.starting_capital,
            position_size_pct: 0.02,
            win_rate: metrics.win_rate / 100.0,
            avg_win_pct: metrics.avg_win_pct,
            avg_loss_pct: metrics.avg_loss_pct,
            win_std_dev: win_std,
            loss_std_dev: loss_std,
        }
    }

    pub fn from_trades(trades: &[TradeResult]) -> Self {
        let metrics = BacktestMetrics::from_trades(trades.to_vec(), 100.0);
        Self::from_metrics(&metrics)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub config: SimulationConfig,
    pub survival_rate: f64,
    pub median_final_capital: f64,
    pub mean_final_capital: f64,
    pub std_final_capital: f64,
    pub percentile_5: f64,
    pub percentile_25: f64,
    pub percentile_75: f64,
    pub percentile_95: f64,
    pub median_max_drawdown: f64,
    pub worst_case_drawdown: f64,
    pub best_case_return: f64,
    pub worst_case_return: f64,
    pub median_return_pct: f64,
    pub risk_of_ruin_pct: f64,
    pub equity_paths: Vec<Vec<f64>>,
    pub final_capitals: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathStatistics {
    pub final_capital: f64,
    pub max_drawdown: f64,
    pub total_return_pct: f64,
    pub survived: bool,
}

pub struct MonteCarloSimulator {
    config: SimulationConfig,
    rng: StdRng,
}

impl MonteCarloSimulator {
    pub fn new(config: SimulationConfig) -> Self {
        Self {
            config,
            rng: StdRng::from_os_rng(),
        }
    }

    pub fn seeded(config: SimulationConfig, seed: u64) -> Self {
        Self {
            config,
            rng: StdRng::seed_from_u64(seed),
        }
    }

    pub fn run(&mut self) -> SimulationResult {
        let mut final_capitals = Vec::with_capacity(self.config.num_simulations);
        let mut max_drawdowns = Vec::with_capacity(self.config.num_simulations);
        let mut equity_paths = Vec::new();

        let ruin_threshold = self.config.starting_capital * 0.10;
        let mut ruin_count = 0;

        let win_dist = Normal::new(self.config.avg_win_pct, self.config.win_std_dev)
            .expect("Invalid win distribution");
        let loss_dist = Normal::new(self.config.avg_loss_pct, self.config.loss_std_dev)
            .expect("Invalid loss distribution");

        for _ in 0..self.config.num_simulations {
            let path_stats = self.simulate_path(&win_dist, &loss_dist, ruin_threshold);

            final_capitals.push(path_stats.final_capital);
            max_drawdowns.push(path_stats.max_drawdown);

            if !path_stats.survived {
                ruin_count += 1;
            }

            if equity_paths.len() < 100 {
                let mut path = vec![self.config.starting_capital];
                let mut capital = self.config.starting_capital;
                let mut cap_count = 0;

                for _ in 0..self.config.num_trades_per_sim {
                    if capital < ruin_threshold {
                        break;
                    }

                    let is_win: bool = self.rng.random::<f64>() < self.config.win_rate;
                    let pnl_pct = if is_win {
                        win_dist.sample(&mut self.rng)
                    } else {
                        loss_dist.sample(&mut self.rng)
                    };

                    let position_size = capital * self.config.position_size_pct;
                    let pnl_usd = pnl_pct * position_size;
                    capital += pnl_usd;
                    capital = capital.max(0.0);

                    path.push(capital);
                    cap_count += 1;

                    if cap_count >= self.config.num_trades_per_sim {
                        break;
                    }
                }

                equity_paths.push(path);
            }
        }

        final_capitals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        max_drawdowns.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let n = final_capitals.len();
        let mean = final_capitals.iter().sum::<f64>() / n as f64;
        let variance = final_capitals
            .iter()
            .map(|c| (c - mean).powi(2))
            .sum::<f64>()
            / n as f64;
        let std = variance.sqrt();

        SimulationResult {
            config: self.config.clone(),
            survival_rate: ((n - ruin_count) as f64 / n as f64) * 100.0,
            median_final_capital: final_capitals[n / 2],
            mean_final_capital: mean,
            std_final_capital: std,
            percentile_5: final_capitals[(n as f64 * 0.05) as usize],
            percentile_25: final_capitals[(n as f64 * 0.25) as usize],
            percentile_75: final_capitals[(n as f64 * 0.75) as usize],
            percentile_95: final_capitals[(n as f64 * 0.95) as usize],
            median_max_drawdown: max_drawdowns[n / 2],
            worst_case_drawdown: *max_drawdowns.last().unwrap_or(&0.0),
            best_case_return: (final_capitals.last().unwrap_or(&0.0)
                / self.config.starting_capital
                - 1.0)
                * 100.0,
            worst_case_return: (final_capitals.first().unwrap_or(&0.0)
                / self.config.starting_capital
                - 1.0)
                * 100.0,
            median_return_pct: ((final_capitals[n / 2] / self.config.starting_capital) - 1.0)
                * 100.0,
            risk_of_ruin_pct: (ruin_count as f64 / n as f64) * 100.0,
            equity_paths,
            final_capitals,
        }
    }

    fn simulate_path(
        &mut self,
        win_dist: &Normal<f64>,
        loss_dist: &Normal<f64>,
        ruin_threshold: f64,
    ) -> PathStatistics {
        let mut capital = self.config.starting_capital;
        let mut peak = capital;
        let mut max_drawdown = 0.0;

        for _ in 0..self.config.num_trades_per_sim {
            if capital < ruin_threshold {
                break;
            }

            let is_win: bool = self.rng.random::<f64>() < self.config.win_rate;

            let pnl_pct = if is_win {
                win_dist.sample(&mut self.rng)
            } else {
                loss_dist.sample(&mut self.rng)
            };

            let position_size = capital * self.config.position_size_pct;
            let pnl_usd = pnl_pct * position_size;
            capital += pnl_usd;
            capital = capital.max(0.0);

            if capital > peak {
                peak = capital;
            }

            let drawdown = (peak - capital) / peak * 100.0;
            if drawdown > max_drawdown {
                max_drawdown = drawdown;
            }
        }

        let survived = capital >= ruin_threshold;
        let total_return = (capital / self.config.starting_capital - 1.0) * 100.0;

        PathStatistics {
            final_capital: capital,
            max_drawdown,
            total_return_pct: total_return,
            survived,
        }
    }

    pub fn run_kelly_analysis(&mut self, max_fraction: f64, steps: usize) -> Vec<KellyResult> {
        let mut results = Vec::new();

        for i in 1..=steps {
            let fraction = (i as f64 / steps as f64) * max_fraction;
            let mut config = self.config.clone();
            config.position_size_pct = fraction;

            let mut sim = MonteCarloSimulator::new(config);
            let result = sim.run();

            results.push(KellyResult {
                kelly_fraction: fraction,
                median_return: result.median_return_pct,
                risk_of_ruin: result.risk_of_ruin_pct,
                median_drawdown: result.median_max_drawdown,
            });
        }

        results
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KellyResult {
    pub kelly_fraction: f64,
    pub median_return: f64,
    pub risk_of_ruin: f64,
    pub median_drawdown: f64,
}

impl SimulationResult {
    pub fn print_summary(&self) {
        println!("\n============ MONTE CARLO RESULTS ============");
        println!("Simulations: {}", self.config.num_simulations);
        println!("Trades per simulation: {}", self.config.num_trades_per_sim);
        println!("Starting Capital: ${:.2}", self.config.starting_capital);
        println!(
            "Position Size: {:.1}% of capital",
            self.config.position_size_pct * 100.0
        );
        println!("--------------------------------------------");
        println!("Input Win Rate: {:.2}%", self.config.win_rate * 100.0);
        println!("Input Avg Win: {:.4}%", self.config.avg_win_pct * 100.0);
        println!("Input Avg Loss: {:.4}%", self.config.avg_loss_pct * 100.0);
        println!("--------------------------------------------");
        println!("=== SURVIVAL ANALYSIS ===");
        println!("Survival Rate: {:.2}%", self.survival_rate);
        println!("Risk of Ruin: {:.2}%", self.risk_of_ruin_pct);
        println!("--------------------------------------------");
        println!("=== CAPITAL DISTRIBUTION ===");
        println!("Median Final: ${:.2}", self.median_final_capital);
        println!("Mean Final: ${:.2}", self.mean_final_capital);
        println!("Std Dev: ${:.2}", self.std_final_capital);
        println!("5th Percentile: ${:.2}", self.percentile_5);
        println!("25th Percentile: ${:.2}", self.percentile_25);
        println!("75th Percentile: ${:.2}", self.percentile_75);
        println!("95th Percentile: ${:.2}", self.percentile_95);
        println!("--------------------------------------------");
        println!("=== RETURNS ===");
        println!("Best Case: {:.2}%", self.best_case_return);
        println!("Worst Case: {:.2}%", self.worst_case_return);
        println!("Median Return: {:.2}%", self.median_return_pct);
        println!("--------------------------------------------");
        println!("=== DRAWDOWN ===");
        println!("Median Max DD: {:.2}%", self.median_max_drawdown);
        println!("Worst Case DD: {:.2}%", self.worst_case_drawdown);
        println!("============================================");
    }

    pub fn export_json(&self, path: &str) -> anyhow::Result<()> {
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }

    pub fn export_equity_paths(&self, path: &str) -> anyhow::Result<()> {
        let mut file = File::create(path)?;
        write!(file, "trade_num")?;

        for i in 0..self.equity_paths.len() {
            write!(file, ",sim_{}", i)?;
        }
        writeln!(file)?;

        let max_len = self.equity_paths.iter().map(|p| p.len()).max().unwrap_or(0);

        for trade_num in 0..max_len {
            write!(file, "{}", trade_num)?;
            for path in &self.equity_paths {
                let val = path.get(trade_num).copied().unwrap_or(0.0);
                write!(file, ",{:.2}", val)?;
            }
            writeln!(file)?;
        }

        Ok(())
    }
}

pub fn print_kelly_analysis(results: &[KellyResult]) {
    println!("\n======== KELLY FRACTION ANALYSIS ========");
    println!(
        "{:<15} {:<15} {:<15} {:<15}",
        "Kelly Frac", "Med Return%", "Risk of Ruin%", "Med DD%"
    );
    println!("{}", "-".repeat(60));

    for r in results {
        println!(
            "{:<15.3} {:<15.2} {:<15.2} {:<15.2}",
            r.kelly_fraction, r.median_return, r.risk_of_ruin, r.median_drawdown
        );
    }
    println!("==========================================");
}
