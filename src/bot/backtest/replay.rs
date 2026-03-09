use crate::bot::backtest::data::{BeckerParser, MarketData, PriceSnapshot};
use crate::bot::backtest::metrics::{BacktestMetrics, ParameterSweepResult, TradeResult};
use crate::bot::candles::CandleEngine;
use crate::bot::indicators::IndicatorEngine;
use crate::bot::signal::{EntrySignal, ExitSignal, SignalEngine};
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Debug, Clone)]
pub struct BacktestConfig {
    pub entry_band_low: f64,
    pub entry_band_high: f64,
    pub slope_threshold: f64,
    pub max_spread_pct: f64,
    pub min_time_remaining: i64,
    pub position_size_usd: f64,
    pub starting_capital: f64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            entry_band_low: 0.35,
            entry_band_high: 0.65,
            slope_threshold: 0.002,
            max_spread_pct: 0.10,
            min_time_remaining: 30,
            position_size_usd: 1.0,
            starting_capital: 100.0,
        }
    }
}

impl BacktestConfig {
    pub fn with_entry_band(low: f64, high: f64) -> Self {
        Self {
            entry_band_low: low,
            entry_band_high: high,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TokenSide {
    Yes,
    No,
}

struct SimPosition {
    side: Option<TokenSide>,
    entry_price: f64,
    entry_timestamp: i64,
    size_usd: f64,
}

impl Default for SimPosition {
    fn default() -> Self {
        Self {
            side: None,
            entry_price: 0.0,
            entry_timestamp: 0,
            size_usd: 0.0,
        }
    }
}

impl SimPosition {
    fn is_active(&self) -> bool {
        self.side.is_some()
    }

    fn pnl(&self, exit_price: f64) -> f64 {
        if !self.is_active() || self.entry_price < 0.0001 {
            return 0.0;
        }
        (exit_price - self.entry_price) / self.entry_price
    }

    fn reset(&mut self) {
        self.side = None;
        self.entry_price = 0.0;
        self.entry_timestamp = 0;
        self.size_usd = 0.0;
    }
}

pub struct BacktestEngine {
    config: BacktestConfig,
    trades: Vec<TradeResult>,
    bankroll: f64,
}

impl BacktestEngine {
    pub fn new(config: BacktestConfig) -> Self {
        Self {
            bankroll: config.starting_capital,
            config,
            trades: Vec::new(),
        }
    }

    pub fn run_market(
        &mut self,
        snapshots: &[PriceSnapshot],
        market_slug: &str,
        resolution: Option<bool>,
    ) -> Vec<TradeResult> {
        if snapshots.len() < 10 {
            return Vec::new();
        }

        let mut candle_engine = CandleEngine::new();
        let mut ind_1m = IndicatorEngine::new();
        let mut ind_5s = IndicatorEngine::new();
        let mut signal_engine =
            SignalEngine::new_with_band(self.config.entry_band_low, self.config.entry_band_high);

        let mut position = SimPosition::default();
        let mut market_trades = Vec::new();

        let end_timestamp = snapshots.last().map(|s| s.timestamp).unwrap_or(0);

        for snapshot in snapshots {
            let epoch_seconds = snapshot.timestamp as u64;
            let time_remaining = end_timestamp - snapshot.timestamp;

            if let Some(closed) = candle_engine.update(
                snapshot.yes_midpoint,
                snapshot.yes_ask - snapshot.yes_bid,
                snapshot.volume,
                epoch_seconds,
            ) {
                if let Some(c) = closed.one_minute {
                    ind_1m.update(&c);
                }
                if let Some(c) = closed.five_second {
                    ind_5s.update(&c);

                    let state_5s = ind_5s.get_state();
                    let state_1m = ind_1m.get_state();

                    let signal = signal_engine.update(&state_5s, &state_1m, snapshot.yes_midpoint);

                    if !position.is_active() && signal.entry != EntrySignal::None {
                        if time_remaining < self.config.min_time_remaining {
                            continue;
                        }

                        let (entry_price, side) = match signal.entry {
                            EntrySignal::Long => (snapshot.yes_ask, TokenSide::Yes),
                            EntrySignal::Short => (snapshot.no_ask, TokenSide::No),
                            EntrySignal::None => continue,
                        };

                        let spread = match signal.entry {
                            EntrySignal::Long => snapshot.yes_ask - snapshot.yes_bid,
                            EntrySignal::Short => snapshot.no_ask - snapshot.no_bid,
                            EntrySignal::None => 0.0,
                        };

                        let max_spread = entry_price * self.config.max_spread_pct;
                        if spread > max_spread {
                            continue;
                        }

                        if entry_price < self.config.entry_band_low
                            || entry_price > self.config.entry_band_high
                        {
                            continue;
                        }

                        if self.bankroll < self.config.position_size_usd {
                            continue;
                        }

                        position.side = Some(side);
                        position.entry_price = entry_price;
                        position.entry_timestamp = snapshot.timestamp;
                        position.size_usd = self.config.position_size_usd;
                        self.bankroll -= self.config.position_size_usd;
                    }

                    if position.is_active() && signal.exit == ExitSignal::FullExit {
                        let exit_price = match position.side {
                            Some(TokenSide::Yes) => snapshot.yes_bid,
                            Some(TokenSide::No) => snapshot.no_bid,
                            None => 0.0,
                        };

                        if exit_price > 0.0001 {
                            let pnl_pct = position.pnl(exit_price);
                            let pnl_usd = pnl_pct * position.size_usd;

                            self.bankroll += position.size_usd + pnl_usd;

                            let side_str = match position.side {
                                Some(TokenSide::Yes) => "YES",
                                Some(TokenSide::No) => "NO",
                                None => "N/A",
                            };

                            market_trades.push(TradeResult {
                                market_slug: market_slug.to_string(),
                                side: side_str.to_string(),
                                entry_price: position.entry_price,
                                exit_price,
                                pnl_percent: pnl_pct,
                                pnl_usd,
                                duration_seconds: snapshot.timestamp - position.entry_timestamp,
                                entry_timestamp: position.entry_timestamp,
                                exit_timestamp: snapshot.timestamp,
                            });

                            position.reset();
                        }
                    }
                }
            }
        }

        if position.is_active() {
            let settlement_price = match resolution {
                Some(true) => match position.side {
                    Some(TokenSide::Yes) => 1.0,
                    Some(TokenSide::No) => 0.0,
                    None => 0.0,
                },
                Some(false) => match position.side {
                    Some(TokenSide::Yes) => 0.0,
                    Some(TokenSide::No) => 1.0,
                    None => 0.0,
                },
                None => {
                    let last = snapshots.last().unwrap();
                    match position.side {
                        Some(TokenSide::Yes) => last.yes_bid,
                        Some(TokenSide::No) => last.no_bid,
                        None => 0.0,
                    }
                }
            };

            let pnl_pct = position.pnl(settlement_price);
            let pnl_usd = pnl_pct * position.size_usd;
            self.bankroll += position.size_usd + pnl_usd;

            let side_str = match position.side {
                Some(TokenSide::Yes) => "YES",
                Some(TokenSide::No) => "NO",
                None => "N/A",
            };

            market_trades.push(TradeResult {
                market_slug: market_slug.to_string(),
                side: side_str.to_string(),
                entry_price: position.entry_price,
                exit_price: settlement_price,
                pnl_percent: pnl_pct,
                pnl_usd,
                duration_seconds: end_timestamp - position.entry_timestamp,
                entry_timestamp: position.entry_timestamp,
                exit_timestamp: end_timestamp,
            });
        }

        self.trades.extend(market_trades.clone());
        market_trades
    }

    pub fn run_all(&mut self, markets: &[MarketData]) -> BacktestMetrics {
        let bar = ProgressBar::new(markets.len() as u64);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );

        for market in markets {
            let snapshots = self.snapshots_from_market(market);
            if !snapshots.is_empty() {
                self.run_market(&snapshots, &market.slug, market.resolution);
            }
            bar.inc(1);
        }

        bar.finish();
        BacktestMetrics::from_trades(self.trades.clone(), self.config.starting_capital)
    }

    fn snapshots_from_market(&self, market: &MarketData) -> Vec<PriceSnapshot> {
        let trades = &market.trades;
        if trades.is_empty() {
            return Vec::new();
        }

        let mut snapshots = Vec::new();

        for trade in trades {
            snapshots.push(PriceSnapshot {
                timestamp: trade.timestamp,
                yes_bid: (trade.price - 0.01_f64).max(0.01_f64),
                yes_ask: (trade.price + 0.01_f64).min(0.99_f64),
                no_bid: (1.0_f64 - trade.price - 0.01_f64).max(0.01_f64),
                no_ask: (1.0_f64 - trade.price + 0.01_f64).min(0.99_f64),
                yes_midpoint: trade.price,
                volume: trade.size,
            });
        }

        snapshots
    }

    pub fn get_trades(&self) -> &[TradeResult] {
        &self.trades
    }

    pub fn get_bankroll(&self) -> f64 {
        self.bankroll
    }

    pub fn reset(&mut self) {
        self.trades.clear();
        self.bankroll = self.config.starting_capital;
    }
}

pub fn run_parameter_sweep(
    markets: &[MarketData],
    bands: &[(f64, f64)],
) -> Vec<ParameterSweepResult> {
    let mut results = Vec::new();

    let bar = ProgressBar::new(bands.len() as u64);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} Band {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    for (low, high) in bands {
        let config = BacktestConfig::with_entry_band(*low, *high);
        let mut engine = BacktestEngine::new(config);
        let metrics = engine.run_all(markets);

        results.push(ParameterSweepResult {
            entry_band_low: *low,
            entry_band_high: *high,
            metrics,
        });

        bar.set_message(format!("{:.2}-{:.2}", low, high));
        bar.inc(1);
    }

    bar.finish();
    results
}
