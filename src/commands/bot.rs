use crate::auth;
use crate::bot::candles::CandleEngine;
use crate::bot::discovery::{
    discover_market_loop, FIVE_MINUTES_SECONDS,
};
use crate::bot::pipeline::{
    self, BacktestArgs, MonteCarloArgs, SweepArgs, FetchPmxtArgs,
    ListMarketsArgs, ExtractMidpointsArgs, InspectParquetArgs, BacktestPipelineArgs,
    BacktestPmxtArgs, run_backtest_pmxt,
};
use crate::bot::feed::{
    LiveFeedMode, LiveStrategyInputSource, StrategyInputSource,
};
use crate::bot::execution::{
    handle_live_signals, get_usdc_balance, try_settle_pending,
    PendingSettlement, LivePosition,
};
use crate::bot::indicators::{IndicatorEngine, IndicatorState};
use crate::bot::logging::{EngineEvent, EngineEventLoggers};
use crate::bot::risk::{best_ask_price, best_bid_price, midpoint_price, GatekeeperState, decimal_to_f64};
use crate::bot::shadow::{ShadowPosition, TokenSide};
use crate::bot::signal::SignalEngine;
use crate::bot::strategy_runner::run_shadow_strategy_step;
use crate::bot::validation::ValidationTracker;
use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::gamma;
use tokio::time::{Duration, MissedTickBehavior, interval};

#[derive(Args)]
pub struct BotArgs {
    #[command(subcommand)]
    pub command: BotCommand,
}

#[derive(Subcommand)]
pub enum BotCommand {
    /// Watch the active 5-minute BTC "Up or Down" market and print live orderbook stats
    WatchBtc(LiveShadowArgs),
    /// Automated 20-market validation run with metrics export
    ValidateBtc(LiveShadowArgs),
    /// LIVE TRADING: Same strategy as watch-btc but places real market orders
    TradeBtc(TradeBtcArgs),
    /// LIVE TRADING: 15-minute BTC markets - same strategy as 5m
    TradeBtc15m(TradeBtcArgs),
    /// Run historical backtest with Becker dataset or mock data
    Backtest(BacktestArgs),
    /// Run Monte Carlo simulation on backtest results
    MonteCarlo(MonteCarloArgs),
    /// Run parameter sweep across entry bands
    Sweep(SweepArgs),
    /// Fetch filtered BTC Up/Down data from remote PMXT Parquet archive
    FetchPmxt(FetchPmxtArgs),
    /// Extract BTC midpoints from local Parquet using Gamma API for ID mapping
    ExtractMidpoints(ExtractMidpointsArgs),
    /// Inspect Parquet file schema and sample data
    InspectParquet(InspectParquetArgs),
    /// List sequential BTC up/down markets from Parquet with 5-min window alignment
    ListMarkets(ListMarketsArgs),
    /// Stream historical orderbook data from PMXT and run shadow backtest
    BacktestPipeline(BacktestPipelineArgs),
    /// Backtest across all PMXT parquet files in a directory with rolling bankroll
    BacktestPmxt(BacktestPmxtArgs),
    /// Export features from PMXT archive to parquet for ML training
    ExportFeatures(ExportFeaturesArgs),
    /// Inspect exported feature parquet file
    InspectFeatures(InspectFeaturesArgs),
    /// Run backtest with Qlib scores
    BacktestScores(BacktestScoresArgs),
    /// Shadow mode with Qlib scores (no live trading)
    ScoreShadow(ScoreShadowArgs),
}

#[derive(Args, Clone)]
pub struct LiveShadowArgs {
    /// Live feed mode for shadow validation
    #[arg(long, value_enum, default_value_t = LiveFeedMode::Websocket)]
    pub feed: LiveFeedMode,

    /// Optional structured event log path or directory
    #[arg(long)]
    pub event_log: Option<String>,

    /// Daily realized loss limit in USD before halting new entries
    #[arg(long, default_value = "2.0")]
    pub daily_loss_limit: f64,

    /// Cooldown duration after a losing trade
    #[arg(long, default_value = "15")]
    pub cooldown_seconds: u64,

    /// Start the engine in an emergency-halted state
    #[arg(long)]
    pub emergency_halt: bool,

    /// Record all orderbook ticks to CSV for later backtesting
    #[arg(long)]
    pub record: bool,

    /// Directory to save recordings (default: recordings/)
    #[arg(long, default_value = "recordings")]
    pub recordings_dir: String,

    /// Strategy mode
    #[arg(long, value_enum, default_value_t = crate::bot::pipeline::BtStrategy::Scalper)]
    pub strategy: crate::bot::pipeline::BtStrategy,
}



// Structs migrated to crate::bot::pipeline

#[derive(Args, Clone)]
pub struct TradeBtcArgs {
    /// Position size in USDC per trade
    #[arg(long, default_value = "1.0")]
    pub size: f64,

    /// Dry run mode - show signals but don't place orders
    #[arg(long)]
    pub dry_run: bool,

    /// Live feed mode for live trading
    #[arg(long, value_enum, default_value_t = LiveFeedMode::Websocket)]
    pub feed: LiveFeedMode,

    /// Optional structured event log path or directory
    #[arg(long)]
    pub event_log: Option<String>,

    /// Daily realized loss limit in USD before halting new entries
    #[arg(long, default_value = "5.0")]
    pub daily_loss_limit: f64,

    /// Cooldown duration after a losing trade
    #[arg(long, default_value = "15")]
    pub cooldown_seconds: u64,

    /// Start the engine in an emergency-halted state
    #[arg(long)]
    pub emergency_halt: bool,
}

// Migrated to crate::bot::pipeline

// Migrated to crate::bot::pipeline

// Migrated to crate::bot::pipeline

// Migrated to crate::bot::pipeline




// Migrated to crate::bot::execution

// ============================================================================
// Research CLI Args
// ============================================================================

#[derive(Args, Clone)]
pub struct ExportFeaturesArgs {
    /// Input PMXT archive path
    #[arg(long)]
    pub input: String,

    /// Output features parquet path
    #[arg(long)]
    pub out: String,

    /// Manifest output path
    #[arg(long)]
    pub manifest_out: Option<String>,

    /// Asset filter (btc, eth, sol, xrp)
    #[arg(long)]
    pub asset: Option<String>,

    /// Duration filter (5m, 15m, 1h)
    #[arg(long)]
    pub duration: Option<String>,

    /// Include labels in output
    #[arg(long)]
    pub with_labels: bool,

    /// Include spot price data
    #[arg(long)]
    pub with_spot: bool,
}

#[derive(Args, Clone)]
pub struct InspectFeaturesArgs {
    /// Input features parquet path
    #[arg(long)]
    pub input: String,

    /// Number of samples to show
    #[arg(long, default_value = "10")]
    pub sample: usize,
}

#[derive(Args, Clone)]
pub struct BacktestScoresArgs {
    /// Input features parquet path
    #[arg(long)]
    pub input: String,

    /// Scores parquet path
    #[arg(long)]
    pub scores: String,

    /// Strategy mode (heuristic, qlib, fused)
    #[arg(long, default_value = "heuristic")]
    pub strategy: String,

    /// Export results path
    #[arg(long)]
    pub export: Option<String>,
}

#[derive(Args, Clone)]
pub struct ScoreShadowArgs {
    /// Scores parquet path
    #[arg(long)]
    pub scores: String,

    /// Asset to trade
    #[arg(long, default_value = "btc")]
    pub asset: String,

    /// Market duration
    #[arg(long, default_value = "5m")]
    pub duration: String,

    /// Strategy mode (heuristic, qlib, fused)
    #[arg(long, default_value = "fused")]
    pub strategy: String,
}

pub async fn execute(args: BotArgs) -> Result<()> {
    match args.command {
        BotCommand::WatchBtc(live_args) => watch_btc_market(None, live_args).await,
        BotCommand::ValidateBtc(live_args) => watch_btc_market(Some(20), live_args).await,
        BotCommand::TradeBtc(trade_args) => trade_btc_live(trade_args).await,
        BotCommand::TradeBtc15m(trade_args) => trade_btc_live(trade_args).await,
        BotCommand::Backtest(backtest_args) => run_backtest(backtest_args).await,
        BotCommand::MonteCarlo(mc_args) => run_monte_carlo(mc_args),
        BotCommand::Sweep(sweep_args) => run_parameter_sweep(sweep_args),
        BotCommand::FetchPmxt(pmxt_args) => run_fetch_pmxt(pmxt_args).await,
        BotCommand::ExtractMidpoints(extract_args) => run_extract_midpoints(extract_args).await,
        BotCommand::InspectParquet(inspect_args) => run_inspect_parquet(inspect_args),
        BotCommand::ListMarkets(list_args) => run_list_markets(list_args).await,
        BotCommand::BacktestPipeline(pipeline_args) => run_backtest_pipeline(pipeline_args).await,
        BotCommand::BacktestPmxt(pmxt_args) => run_backtest_pmxt(pmxt_args).await,
        BotCommand::ExportFeatures(export_args) => run_export_features(export_args).await,
        BotCommand::InspectFeatures(inspect_args) => run_inspect_features(inspect_args),
        BotCommand::BacktestScores(backtest_args) => run_backtest_scores(backtest_args).await,
        BotCommand::ScoreShadow(shadow_args) => run_score_shadow(shadow_args).await,
    }
}

fn create_event_loggers(path: Option<&str>) -> Result<Option<EngineEventLoggers>> {
    path.map(EngineEventLoggers::new)
        .transpose()
        .context("Failed to create structured event logs")
}

async fn create_live_input_source<'a>(
    feed_mode: LiveFeedMode,
    client: &'a clob::Client,
    watched: &crate::bot::discovery::WatchedMarket,
) -> Result<LiveStrategyInputSource<'a>> {
    match feed_mode {
        LiveFeedMode::Poll => Ok(LiveStrategyInputSource::poll(
            client,
            watched.yes_token_id,
            watched.no_token_id,
        )),
        LiveFeedMode::Websocket => {
            LiveStrategyInputSource::websocket(
                watched.condition_id.clone(),
                watched.yes_token_id,
                watched.no_token_id,
                None,
            )
            .await
        }
    }
}

async fn watch_btc_market(max_markets: Option<usize>, live_args: LiveShadowArgs) -> Result<()> {
    let gamma_client = gamma::Client::default();
    let clob_client = clob::Client::default();
    let event_loggers = create_event_loggers(live_args.event_log.as_deref())?;

    let mut watched = discover_market_loop(&gamma_client).await;
    let mut input_source = create_live_input_source(live_args.feed, &clob_client, &watched)
        .await
        .context("Failed to create live strategy input source")?;

    let mut validator = max_markets.map(ValidationTracker::new);

    let mut ind_1m = IndicatorEngine::new();
    let mut ind_5s = IndicatorEngine::new();

    let (band_low, band_high) = match live_args.strategy {
        crate::bot::pipeline::BtStrategy::Scalper => (0.35, 0.65),
        crate::bot::pipeline::BtStrategy::LateWindow => (0.85, 0.98),
        crate::bot::pipeline::BtStrategy::FairValue => (0.05, 0.95),
        crate::bot::pipeline::BtStrategy::HawkesFlow => (0.05, 0.95),
        crate::bot::pipeline::BtStrategy::BookValue => (0.05, 0.95),
        crate::bot::pipeline::BtStrategy::CandleClock => (0.05, 0.95),
    };
    let mut signal_engine = SignalEngine::new_with_band(band_low, band_high);

    let mut candle_engine = CandleEngine::new();
    candle_engine.set_debug(false);

    let mut shadow = ShadowPosition::default();
    let mut gatekeeper =
        GatekeeperState::new(live_args.daily_loss_limit, live_args.cooldown_seconds);
    if live_args.emergency_halt {
        gatekeeper.halt();
    }

    let mut state_1m = IndicatorState::default();
    let mut state_5s = IndicatorState::default();

    let mut last_yes_bid = 0.0;
    let mut last_no_bid = 0.0;
    let mut current_slug = watched.slug.clone();

    // FairValue model state (created per market)
    let mut fv_model = crate::bot::pricing::LogitJumpDiffusion::new();
    let mut fv_kalman = Some(crate::bot::pricing::KalmanFilter::new(0.0));
    let mut fv_jump_calibrator = Some(crate::bot::pricing::JumpCalibrator::with_defaults());
    let mut fv_calibrated = {
        let base_model = crate::bot::pricing::FairValueModel::default();
        crate::bot::pricing::CalibratedFairValue::with_defaults(base_model)
    };
    let mut fv_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
    let mut fv_cumulative_wins: Vec<f64> = Vec::new();
    let mut fv_cumulative_losses: Vec<f64> = Vec::new();

    // HawkesFlow state
    let mut hawkes_engine = crate::bot::strategy::HawkesFlowEngine::new();
    let mut hawkes_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
    let mut hawkes_cumulative_wins: Vec<f64> = Vec::new();
    let mut hawkes_cumulative_losses: Vec<f64> = Vec::new();

    // BookValue state
    let mut bookvalue_engine = crate::bot::strategy::BookValueEngine::new();
    let mut bookvalue_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
    let mut bookvalue_cumulative_wins: Vec<f64> = Vec::new();
    let mut bookvalue_cumulative_losses: Vec<f64> = Vec::new();

    // CandleClock state
    let mut clock_engine = crate::bot::strategy::CandleClockEngine::new();
    let mut clock_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
    let mut clock_cumulative_wins: Vec<f64> = Vec::new();
    let mut clock_cumulative_losses: Vec<f64> = Vec::new();

    let mut ticker = interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    println!("[SHADOW MODE] Strategy: {:?}", live_args.strategy);
    println!("[SHADOW MODE] Entry bands: {:.2} - {:.2}", band_low, band_high);
    println!("[SHADOW MODE] Size: $1.00 | Feed: {:?}", live_args.feed);
    println!("========================================");

    // Create tick recorder if --record flag is set
    let mut recorder = if live_args.record {
        Some(crate::bot::recording::TickRecorder::new(&live_args.recordings_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create recorder: {}", e))?)
    } else {
        None
    };

    let is_ws = input_source.is_websocket();
    if is_ws {
        println!("[FEED] Event-by-event WebSocket mode");
    } else {
        println!("[FEED] 1Hz polling mode");
    }

    'market_loop: loop {
    if is_ws {
        // ─── Event-by-event WebSocket loop ───
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\n[SHADOW] Final PnL: {:.4}%", shadow.realized_pnl * 100.0);
                    if let Some(v) = &validator {
                        v.print_summary();
                    }
                    println!("Received Ctrl+C, stopping bot watch.");
                    break 'market_loop;
                }
                result = input_source.recv_next() => {
                    let dual_snapshot = match result {
                        Some(snap) => snap,
                        None => {
                            eprintln!("[WS] Feed disconnected, reconnecting...");
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                    };

                    last_yes_bid = best_bid_price(&dual_snapshot.yes).unwrap_or(last_yes_bid);
                    last_no_bid = best_bid_price(&dual_snapshot.no).unwrap_or(last_no_bid);
                    let epoch_seconds = (dual_snapshot.ts_exchange.floor() as u64) / 1000;

                    // Market expiry check (based on exchange timestamp)
                    if epoch_seconds as i64 >= watched.end_time.timestamp() || watched.slug != current_slug {
                        if shadow.is_active() {
                            let price = match shadow.token_side {
                                Some(TokenSide::Yes) => last_yes_bid,
                                Some(TokenSide::No) => last_no_bid,
                                None => shadow.entry_price,
                            };
                            let settlement_price = if price > 0.5 { 1.0 } else { 0.0 };
                            let final_pnl = shadow.pnl(settlement_price);
                            shadow.realized_pnl += final_pnl * shadow.size;
                            shadow.position_realized_pnl += final_pnl * shadow.size;
                            let dollar_pnl = final_pnl * shadow.position_size_usd;
                            shadow.bankroll_usd += shadow.position_size_usd + dollar_pnl;
                            shadow.realized_usd += dollar_pnl;
                            shadow.position_realized_usd += dollar_pnl;
                            if let Some(v) = &mut validator {
                                let duration = (Utc::now().timestamp() as u64 - shadow.entry_timestamp) as i64;
                                let side_str = match shadow.token_side { Some(TokenSide::Yes) => "YES".to_string(), Some(TokenSide::No) => "NO".to_string(), None => "N/A".to_string() };
                                v.record_trade(watched.slug.clone(), side_str, shadow.entry_price, settlement_price, shadow.position_realized_pnl, duration, shadow.position_realized_usd, shadow.bankroll_usd);
                            }
                            let side_name = match shadow.token_side { Some(TokenSide::Yes) => "YES", Some(TokenSide::No) => "NO", None => "N/A" };
                            println!("[SETTLEMENT] {} | {} @ {:.2} -> {:.2} | {:.4}% | Bankroll: ${:.2}",
                                side_name, watched.slug, shadow.entry_price, settlement_price, shadow.position_realized_pnl * 100.0, shadow.bankroll_usd);
                        }
                        if let Some(v) = &mut validator {
                            v.finalize_market(watched.slug.clone(), shadow.realized_pnl);
                            if v.completed_markets >= v.max_markets { v.print_summary(); return Ok(()); }
                        }
                        println!("Market {} reached resolution time. Looking for next active BTC 5m market...", watched.slug);
                        watched = discover_market_loop(&gamma_client).await;
                        current_slug = watched.slug.clone();
                        input_source.shutdown().await;
                        input_source = create_live_input_source(live_args.feed, &clob_client, &watched).await
                            .context("Failed to recreate live strategy input source")?;
                        signal_engine.reset(); ind_1m.reset(); ind_5s.reset(); shadow.full_reset();
                        fv_model = crate::bot::pricing::LogitJumpDiffusion::new();
                        fv_kalman = Some(crate::bot::pricing::KalmanFilter::new(0.0));
                        fv_jump_calibrator = Some(crate::bot::pricing::JumpCalibrator::with_defaults());
                        fv_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                        hawkes_engine = crate::bot::strategy::HawkesFlowEngine::new();
                        hawkes_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                        bookvalue_engine = crate::bot::strategy::BookValueEngine::new();
                        bookvalue_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                        clock_engine = crate::bot::strategy::CandleClockEngine::new();
                        clock_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                        state_1m = IndicatorState::default(); state_5s = IndicatorState::default();
                        last_yes_bid = 0.0; last_no_bid = 0.0;
                        println!("[MARKET RESET] All engines cleared | {}", watched.slug);
                        println!("========================================");
                        continue 'market_loop;
                    }

                    // Record tick
                    if let Some(ref mut rec) = recorder {
                        let time_remaining = (watched.end_time.timestamp() - epoch_seconds as i64).max(0);
                        rec.record_tick(dual_snapshot.ts_exchange, &watched.slug, last_yes_bid,
                            best_ask_price(&dual_snapshot.yes).unwrap_or(0.0), last_no_bid,
                            best_ask_price(&dual_snapshot.no).unwrap_or(0.0), time_remaining);
                    }

                    // Status print (every ~10s based on timestamp)
                    if epoch_seconds % 10 == 0 {
                        let yb = best_bid_price(&dual_snapshot.yes).unwrap_or(0.0);
                        let ya = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
                        let nb = best_bid_price(&dual_snapshot.no).unwrap_or(0.0);
                        let na = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);
                        println!("[BOOK] YES: bid={:.4} ask={:.4} | NO: bid={:.4} ask={:.4} | mid={:.4}",
                            yb, ya, nb, na, midpoint_price(&dual_snapshot.yes).unwrap_or(0.0));
                    }

                    // ─── Strategy processing ───
                    if live_args.strategy == crate::bot::pipeline::BtStrategy::FairValue {
                        let prev_trades = fv_metrics.trades_taken;
                        crate::bot::pipeline::process_fairvalue_snapshot(
                            &mut fv_model, &mut fv_kalman, &mut fv_jump_calibrator, &mut fv_calibrated,
                            &dual_snapshot, epoch_seconds, watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(), &mut shadow, &mut fv_metrics, 1.0, 0.05, true,
                            &mut fv_cumulative_wins, &mut fv_cumulative_losses);
                        if fv_metrics.trades_taken > prev_trades { if let Some(v) = &mut validator { v.record_signal(); v.record_entry_taken(); } }
                    } else if live_args.strategy == crate::bot::pipeline::BtStrategy::HawkesFlow {
                        let prev_trades = hawkes_metrics.trades_taken;
                        crate::bot::pipeline::process_hawkesflow_snapshot(
                            &mut hawkes_engine, &dual_snapshot, epoch_seconds, watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(), &mut shadow, &mut hawkes_metrics, 1.0, true,
                            &mut hawkes_cumulative_wins, &mut hawkes_cumulative_losses);
                        if hawkes_metrics.trades_taken > prev_trades { if let Some(v) = &mut validator { v.record_signal(); v.record_entry_taken(); } }
                    } else if live_args.strategy == crate::bot::pipeline::BtStrategy::BookValue {
                        let prev_trades = bookvalue_metrics.trades_taken;
                        crate::bot::pipeline::process_bookvalue_snapshot(
                            &mut bookvalue_engine, &dual_snapshot, epoch_seconds, watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(), &mut shadow, &mut bookvalue_metrics, 1.0, true,
                            &mut bookvalue_cumulative_wins, &mut bookvalue_cumulative_losses);
                        if bookvalue_metrics.trades_taken > prev_trades { if let Some(v) = &mut validator { v.record_signal(); v.record_entry_taken(); } }
                    } else if live_args.strategy == crate::bot::pipeline::BtStrategy::CandleClock {
                        let prev_trades = clock_metrics.trades_taken;
                        crate::bot::pipeline::process_candleclock_snapshot(
                            &mut clock_engine, &dual_snapshot, epoch_seconds, watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(), &mut shadow, &mut clock_metrics, 1.0, true,
                            &mut clock_cumulative_wins, &mut clock_cumulative_losses);
                        if clock_metrics.trades_taken > prev_trades { if let Some(v) = &mut validator { v.record_signal(); v.record_entry_taken(); } }
                    } else {
                        if let Some(step) = run_shadow_strategy_step(
                            &dual_snapshot, &watched.label, &watched.slug,
                            watched.end_time.timestamp() - FIVE_MINUTES_SECONDS, watched.end_time.timestamp(),
                            epoch_seconds, 1.0, &mut candle_engine, &mut ind_1m, &mut ind_5s,
                            &mut signal_engine, &mut state_1m, &mut state_5s, &mut shadow, &mut gatekeeper,
                            event_loggers.as_ref()) {
                            if step.signal_seen { if let Some(v) = &mut validator { v.record_signal(); } }
                            if step.entry_blocked { if let Some(v) = &mut validator { v.record_entry_blocked(); } }
                            if step.entry_taken { if let Some(v) = &mut validator { v.record_entry_taken(); } }
                            if let Some(exit_trade) = step.exit_trade {
                                if let Some(v) = &mut validator {
                                    v.record_trade(watched.slug.clone(), exit_trade.side, exit_trade.entry_price, exit_trade.exit_price, exit_trade.pnl_pct, exit_trade.duration, exit_trade.pnl_usd, exit_trade.bankroll_after);
                                }
                            }
                        }
                    }

                    // Position status
                    if shadow.is_active() {
                        let exit_price = match shadow.token_side {
                            Some(TokenSide::Yes) => best_bid_price(&dual_snapshot.yes).unwrap_or(0.0),
                            Some(TokenSide::No) => best_bid_price(&dual_snapshot.no).unwrap_or(0.0),
                            _ => 0.0,
                        };
                        let unrealized = shadow.pnl(exit_price) * shadow.size;
                        let total = shadow.realized_pnl + unrealized;
                        if epoch_seconds % 30 == 0 {
                            println!("[TICK] {:?} entry={:.4} current={:.4} | PnL: {:.4}% | Total: {:.4}%",
                                shadow.token_side.unwrap(), shadow.entry_price, exit_price, unrealized * 100.0, total * 100.0);
                        }
                    }
                }
            }
        }
    } else {
        // ─── Polling loop (1Hz ticker) ───
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\n[SHADOW] Final PnL: {:.4}%", shadow.realized_pnl * 100.0);
                    if let Some(v) = &validator {
                        v.print_summary();
                    }
                    println!("Received Ctrl+C, stopping bot watch.");
                    break;
                }
                _ = ticker.tick() => {
                if Utc::now() >= watched.end_time || watched.slug != current_slug {
                    if shadow.is_active() {
                        let price = match shadow.token_side {
                            Some(TokenSide::Yes) => last_yes_bid,
                            Some(TokenSide::No) => last_no_bid,
                            None => shadow.entry_price,
                        };

                        let settlement_price = if price > 0.5 { 1.0 } else { 0.0 };
                        let final_pnl = shadow.pnl(settlement_price);
                        shadow.realized_pnl += final_pnl * shadow.size;
                        shadow.position_realized_pnl += final_pnl * shadow.size;

                        let dollar_pnl = final_pnl * shadow.position_size_usd;
                        shadow.bankroll_usd += shadow.position_size_usd + dollar_pnl;
                        shadow.realized_usd += dollar_pnl;
                        shadow.position_realized_usd += dollar_pnl;

                        if let Some(v) = &mut validator {
                            let duration = (Utc::now().timestamp() as u64 - shadow.entry_timestamp) as i64;
                            let side_str = match shadow.token_side {
                                Some(TokenSide::Yes) => "YES".to_string(),
                                Some(TokenSide::No) => "NO".to_string(),
                                None => "N/A".to_string(),
                            };
                            v.record_trade(
                                watched.slug.clone(),
                                side_str,
                                shadow.entry_price,
                                settlement_price,
                                shadow.position_realized_pnl,
                                duration,
                                shadow.position_realized_usd,
                                shadow.bankroll_usd,
                            );
                        }

                        let side_name = match shadow.token_side {
                            Some(TokenSide::Yes) => "YES",
                            Some(TokenSide::No) => "NO",
                            None => "N/A",
                        };
                        println!(
                            "[SETTLEMENT] {} | {} @ {:.2} -> {:.2} | {:.4}% | Bankroll: ${:.2}",
                            side_name,
                            watched.slug,
                            shadow.entry_price,
                            settlement_price,
                            shadow.position_realized_pnl * 100.0,
                            shadow.bankroll_usd
                        );
                    }

                    if let Some(v) = &mut validator {
                        v.finalize_market(watched.slug.clone(), shadow.realized_pnl);
                        if v.completed_markets >= v.max_markets {
                            v.print_summary();
                            return Ok(());
                        }
                    }

                    println!(
                        "Market {} reached resolution time. Looking for next active BTC 5m market...",
                        watched.slug
                    );
                    watched = discover_market_loop(&gamma_client).await;
                    current_slug = watched.slug.clone();
                    input_source.shutdown().await;
                    input_source = create_live_input_source(live_args.feed, &clob_client, &watched)
                        .await
                        .context("Failed to recreate live strategy input source")?;

                    signal_engine.reset();
                    ind_1m.reset();
                    ind_5s.reset();
                    shadow.full_reset();
                    fv_model = crate::bot::pricing::LogitJumpDiffusion::new();
                    fv_kalman = Some(crate::bot::pricing::KalmanFilter::new(0.0));
                    fv_jump_calibrator = Some(crate::bot::pricing::JumpCalibrator::with_defaults());
                    fv_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                    hawkes_engine = crate::bot::strategy::HawkesFlowEngine::new();
                    hawkes_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                    bookvalue_engine = crate::bot::strategy::BookValueEngine::new();
                    bookvalue_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                    clock_engine = crate::bot::strategy::CandleClockEngine::new();
                    clock_metrics = crate::bot::pipeline::PipelineMetrics::new(shadow.bankroll_usd);
                    state_1m = IndicatorState::default();
                    state_5s = IndicatorState::default();
                    last_yes_bid = 0.0;
                    last_no_bid = 0.0;

                    println!("[MARKET RESET] All engines cleared | {}", watched.slug);
                    println!("========================================");
                }

                let dual_snapshot = match input_source.next_snapshot().await {
                    Ok(Some(snapshot)) => snapshot,
                    Ok(None) => continue,
                    Err(err) => {
                        eprintln!("[warn] Failed to fetch live snapshot: {err:#}");
                        continue;
                    }
                };

                if let Some(midpoint) = midpoint_price(&dual_snapshot.yes) {
                    last_yes_bid = best_bid_price(&dual_snapshot.yes).unwrap_or(last_yes_bid);
                    last_no_bid = best_bid_price(&dual_snapshot.no).unwrap_or(last_no_bid);
                    let epoch_seconds = (input_source
                        .current_time()
                        .unwrap_or_else(|| dual_snapshot.ts_exchange.floor() as u64)) / 1000;

                    // Record tick if recording is enabled
                    if let Some(ref mut rec) = recorder {
                        let time_remaining = (watched.end_time.timestamp() - epoch_seconds as i64).max(0);
                        rec.record_tick(
                            dual_snapshot.ts_exchange,
                            &watched.slug,
                            last_yes_bid,
                            best_ask_price(&dual_snapshot.yes).unwrap_or(0.0),
                            last_no_bid,
                            best_ask_price(&dual_snapshot.no).unwrap_or(0.0),
                            time_remaining,
                        );
                    }

                    if let Some(loggers) = &event_loggers {
                        loggers.log_market(EngineEvent::BookUpdate {
                            ts: epoch_seconds,
                            market_slug: watched.slug.clone(),
                            source: format!("{:?}", live_args.feed),
                            yes_bid: best_bid_price(&dual_snapshot.yes).unwrap_or(0.0),
                            yes_ask: best_ask_price(&dual_snapshot.yes).unwrap_or(0.0),
                            no_bid: best_bid_price(&dual_snapshot.no).unwrap_or(0.0),
                            no_ask: best_ask_price(&dual_snapshot.no).unwrap_or(0.0),
                        });
                    }

                    if epoch_seconds % 10 == 0 {
                        let yes_bid = best_bid_price(&dual_snapshot.yes).unwrap_or(0.0);
                        let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
                        let no_bid = best_bid_price(&dual_snapshot.no).unwrap_or(0.0);
                        let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);
                        let yes_spread = yes_ask - yes_bid;
                        let no_spread = no_ask - no_bid;
                        let yes_max = yes_ask * 0.10;
                        let no_max = no_ask * 0.10;
                        println!("[BOOK] YES: bid={:.4} ask={:.4} spread={:.4} max={:.4} | NO: bid={:.4} ask={:.4} spread={:.4} max={:.4} | mid={:.4}",
                            yes_bid, yes_ask, yes_spread, yes_max, no_bid, no_ask, no_spread, no_max, midpoint);
                    }

                    if live_args.strategy == crate::bot::pipeline::BtStrategy::FairValue {
                        // FairValue: use logit jump-diffusion model
                        let prev_trades = fv_metrics.trades_taken;
                        crate::bot::pipeline::process_fairvalue_snapshot(
                            &mut fv_model,
                            &mut fv_kalman,
                            &mut fv_jump_calibrator,
                            &mut fv_calibrated,
                            &dual_snapshot,
                            epoch_seconds,
                            watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(),
                            &mut shadow,
                            &mut fv_metrics,
                            1.0,
                            0.05,
                            true,
                            &mut fv_cumulative_wins,
                            &mut fv_cumulative_losses,
                        );
                        // Update validator if trade happened
                        if fv_metrics.trades_taken > prev_trades {
                            if let Some(v) = &mut validator {
                                v.record_signal();
                                v.record_entry_taken();
                            }
                        }
                    } else if live_args.strategy == crate::bot::pipeline::BtStrategy::HawkesFlow {
                        let prev_trades = hawkes_metrics.trades_taken;
                        crate::bot::pipeline::process_hawkesflow_snapshot(
                            &mut hawkes_engine,
                            &dual_snapshot,
                            epoch_seconds,
                            watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(),
                            &mut shadow,
                            &mut hawkes_metrics,
                            1.0,
                            true,
                            &mut hawkes_cumulative_wins,
                            &mut hawkes_cumulative_losses,
                        );
                        if hawkes_metrics.trades_taken > prev_trades {
                            if let Some(v) = &mut validator {
                                v.record_signal();
                                v.record_entry_taken();
                            }
                        }
                    } else if live_args.strategy == crate::bot::pipeline::BtStrategy::BookValue {
                        let prev_trades = bookvalue_metrics.trades_taken;
                        crate::bot::pipeline::process_bookvalue_snapshot(
                            &mut bookvalue_engine,
                            &dual_snapshot,
                            epoch_seconds,
                            watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(),
                            &mut shadow,
                            &mut bookvalue_metrics,
                            1.0,
                            true,
                            &mut bookvalue_cumulative_wins,
                            &mut bookvalue_cumulative_losses,
                        );
                        if bookvalue_metrics.trades_taken > prev_trades {
                            if let Some(v) = &mut validator {
                                v.record_signal();
                                v.record_entry_taken();
                            }
                        }
                    } else if live_args.strategy == crate::bot::pipeline::BtStrategy::CandleClock {
                        let prev_trades = clock_metrics.trades_taken;
                        crate::bot::pipeline::process_candleclock_snapshot(
                            &mut clock_engine,
                            &dual_snapshot,
                            epoch_seconds,
                            watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                            watched.end_time.timestamp(),
                            &mut shadow,
                            &mut clock_metrics,
                            1.0,
                            true,
                            &mut clock_cumulative_wins,
                            &mut clock_cumulative_losses,
                        );
                        if clock_metrics.trades_taken > prev_trades {
                            if let Some(v) = &mut validator {
                                v.record_signal();
                                v.record_entry_taken();
                            }
                        }
                    } else {
                    if let Some(step) = run_shadow_strategy_step(
                        &dual_snapshot,
                        &watched.label,
                        &watched.slug,
                        watched.end_time.timestamp() - FIVE_MINUTES_SECONDS,
                        watched.end_time.timestamp(),
                        epoch_seconds,
                        1.0,
                        &mut candle_engine,
                        &mut ind_1m,
                        &mut ind_5s,
                        &mut signal_engine,
                        &mut state_1m,
                        &mut state_5s,
                        &mut shadow,
                        &mut gatekeeper,
                        event_loggers.as_ref(),
                    ) {
                        if step.signal_seen {
                            if let Some(v) = &mut validator {
                                v.record_signal();
                            }
                        }
                        if step.entry_blocked {
                            if let Some(v) = &mut validator {
                                v.record_entry_blocked();
                            }
                        }
                        if step.entry_taken {
                            if let Some(v) = &mut validator {
                                v.record_entry_taken();
                            }
                        }
                        if let Some(exit_trade) = step.exit_trade {
                            if let Some(v) = &mut validator {
                                v.record_trade(
                                    watched.slug.clone(),
                                    exit_trade.side,
                                    exit_trade.entry_price,
                                    exit_trade.exit_price,
                                    exit_trade.pnl_pct,
                                    exit_trade.duration,
                                    exit_trade.pnl_usd,
                                    exit_trade.bankroll_after,
                                );
                            }
                        }
                    }
                    } // end else (scalper/late-window strategy)

                    if shadow.is_active() {
                        let exit_price = match shadow.token_side {
                            Some(TokenSide::Yes) => best_bid_price(&dual_snapshot.yes).unwrap_or(0.0),
                            Some(TokenSide::No) => best_bid_price(&dual_snapshot.no).unwrap_or(0.0),
                            _ => 0.0,
                        };

                        let unrealized = shadow.pnl(exit_price) * shadow.size;
                        let total = shadow.realized_pnl + unrealized;
                        if epoch_seconds % 30 == 0 {
                            let yes_bid = best_bid_price(&dual_snapshot.yes).unwrap_or(0.0);
                            let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
                            let no_bid = best_bid_price(&dual_snapshot.no).unwrap_or(0.0);
                            let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);

                            println!(
                                "[BOOK] YES {:.4}/{:.4} | NO {:.4}/{:.4} | sum={:.4}",
                                yes_bid, yes_ask, no_bid, no_ask, yes_ask + no_ask
                            );
                            println!(
                                "[TICK] {:?} entry={:.4} current={:.4} | PnL: {:.4}% | Total: {:.4}%",
                                shadow.token_side.unwrap(), shadow.entry_price, exit_price, unrealized * 100.0, total * 100.0
                            );
                        }
                    }
                }
            }
        } // end poll select
    } // end poll loop
    } // end else
    } // end market_loop

    input_source.shutdown().await;

    Ok(())
}

// Migrated to crate::bot::execution

async fn trade_btc_live(args: TradeBtcArgs) -> Result<()> {
    let signer = auth::resolve_signer(None)?;
    let clob_client = auth::authenticate_with_signer(&signer, None).await?;
    let gamma_client = gamma::Client::default();
    let read_client = clob::Client::default();
    let event_loggers = create_event_loggers(args.event_log.as_deref())?;

    let balance = get_usdc_balance(&clob_client).await?;
    println!("[LIVE] USDC Balance: ${:.2}", balance);
    
    if balance < args.size {
        anyhow::bail!("Insufficient USDC balance: ${:.2} < ${:.2}", balance, args.size);
    }

    let mut watched = discover_market_loop(&gamma_client).await;
    let mut input_source = create_live_input_source(args.feed, &read_client, &watched)
        .await
        .context("Failed to create live trading input source")?;

    let mut ind_1m = IndicatorEngine::new();
    let mut ind_5s = IndicatorEngine::new();
    let mut signal_engine = SignalEngine::new();
    let mut candle_engine = CandleEngine::new();
    candle_engine.set_debug(false);

    let mut position = LivePosition::default();
    let mut gatekeeper = GatekeeperState::new(args.daily_loss_limit, args.cooldown_seconds);
    if args.emergency_halt {
        gatekeeper.halt();
    }
    let mut state_1m = IndicatorState::default();
    let mut state_5s = IndicatorState::default();
    let mut current_slug = watched.slug.clone();

    let mut pending_settlements: Vec<PendingSettlement> = Vec::new();

    let mut ticker = interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    if args.dry_run {
        println!("[LIVE *** DRY RUN ***] No orders will be placed");
    }
    println!("[LIVE] Probability Expansion Scalper");
    println!("[LIVE] Entry: slope > 0.002 + breakout | Exit: slope flip");
    println!("[LIVE] Range: 0.35 - 0.65 | Size: ${:.2}", args.size);
    println!("[LIVE] Auto-sell enabled for pending positions");
    println!("[LIVE] Feed: {:?}", args.feed);
    println!("========================================");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n[LIVE] Stopping bot...");
                if position.is_active() {
                    println!("[LIVE] WARNING: Position still open! Manual exit required.");
                }
                if !pending_settlements.is_empty() {
                    println!("[LIVE] WARNING: {} pending settlements require manual resolution!", pending_settlements.len());
                    for p in &pending_settlements {
                        println!("  - {} | {:?} | {} shares", p.market_slug, p.token_side, p.shares);
                    }
                }
                break;
            }
            _ = ticker.tick() => {
                let now = Utc::now();

                // Try to settle pending positions
                if !pending_settlements.is_empty() && !args.dry_run {
                    try_settle_pending(
                        &mut pending_settlements,
                        &read_client,
                        &clob_client,
                        &signer,
                        &mut gatekeeper,
                        event_loggers.as_ref(),
                        now,
                    ).await;
                }

                // Check if current market ended
                if now >= watched.end_time || watched.slug != current_slug {
                    // Move active position to pending settlements
                    if position.is_active() {
                        let (token_id, token_side) = match position.token_side {
                            Some(TokenSide::Yes) => (watched.yes_token_id, TokenSide::Yes),
                            Some(TokenSide::No) => (watched.no_token_id, TokenSide::No),
                            None => {
                                position.full_reset();
                                return Ok(());
                            }
                        };

                        let pending = PendingSettlement {
                            market_slug: watched.slug.clone(),
                            token_side,
                            token_id,
                            shares: position.shares,
                            entry_price: position.entry_price,
                            condition_id: watched.condition_id.clone(),
                            end_time: watched.end_time,
                            sell_attempts: 0,
                            created_at: now,
                        };

                        println!(
                            "[PENDING] {} | {:?} | {:.4} shares @ {:.4} | Auto-sell queued",
                            pending.market_slug, pending.token_side, pending.shares, pending.entry_price
                        );

                        pending_settlements.push(pending);
                    }

                    // Always full reset position when market ends (clears directional locks)
                    position.full_reset();

                    println!("[LIVE] Market {} ended. Looking for next market...", watched.slug);
                    watched = discover_market_loop(&gamma_client).await;
                    current_slug = watched.slug.clone();
                    input_source.shutdown().await;
                    input_source = create_live_input_source(args.feed, &read_client, &watched)
                        .await
                        .context("Failed to recreate live trading input source")?;

                    signal_engine.reset();
                    ind_1m.reset();
                    ind_5s.reset();
                    state_1m = IndicatorState::default();
                    state_5s = IndicatorState::default();

                    println!("[MARKET RESET] All engines cleared | {}", watched.slug);
                    if !pending_settlements.is_empty() {
                        println!("[PENDING] {} positions awaiting settlement", pending_settlements.len());
                    }
                    println!("========================================");
                    continue;
                }

                let dual_snapshot = match input_source.next_snapshot().await {
                    Ok(Some(snapshot)) => snapshot,
                    Ok(None) => continue,
                    Err(err) => {
                        eprintln!("[warn] Failed to fetch live trading snapshot: {err:#}");
                        continue;
                    }
                };

                if let Some(midpoint) = midpoint_price(&dual_snapshot.yes) {
                    let spread_f64 = dual_snapshot.yes.spread.map(decimal_to_f64).unwrap_or(0.0);
                    let simulated_volume = decimal_to_f64(dual_snapshot.yes.top5_bid_depth + dual_snapshot.yes.top5_ask_depth);
                    let epoch_seconds = input_source.current_time().unwrap_or(now.timestamp() as u64);

                    if let Some(loggers) = &event_loggers {
                        loggers.log_market(EngineEvent::BookUpdate {
                            ts: epoch_seconds,
                            market_slug: watched.slug.clone(),
                            source: format!("{:?}", args.feed),
                            yes_bid: best_bid_price(&dual_snapshot.yes).unwrap_or(0.0),
                            yes_ask: best_ask_price(&dual_snapshot.yes).unwrap_or(0.0),
                            no_bid: best_bid_price(&dual_snapshot.no).unwrap_or(0.0),
                            no_ask: best_ask_price(&dual_snapshot.no).unwrap_or(0.0),
                        });
                    }

                    if epoch_seconds % 10 == 0 {
                        let yes_bid = best_bid_price(&dual_snapshot.yes).unwrap_or(0.0);
                        let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
                        let no_bid = best_bid_price(&dual_snapshot.no).unwrap_or(0.0);
                        let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);
                        println!("[BOOK] YES: {:.4}/{:.4} | NO: {:.4}/{:.4} | mid={:.4}", 
                            yes_bid, yes_ask, no_bid, no_ask, midpoint);
                    }

                    let closed_candles = candle_engine.update(midpoint, spread_f64, simulated_volume, epoch_seconds);

                    if let Some(closed) = closed_candles {
                        if let Some(c) = closed.one_minute {
                            state_1m = ind_1m.update(&c);
                        }

                        if let Some(c) = closed.five_second {
                            state_5s = ind_5s.update(&c);

                            let signal = signal_engine.update(&state_5s, &state_1m, midpoint);
                            if let Some(loggers) = &event_loggers {
                                loggers.log_strategy(EngineEvent::StrategySignal {
                                    ts: epoch_seconds,
                                    market_slug: watched.slug.clone(),
                                    midpoint,
                                    entry: format!("{:?}", signal.entry),
                                    exit: format!("{:?}", signal.exit),
                                    detail: "live_signal_engine".to_string(),
                                });
                            }

                            handle_live_signals(
                                &signal,
                                &dual_snapshot,
                                &mut position,
                                &mut gatekeeper,
                                event_loggers.as_ref(),
                                &watched,
                                epoch_seconds,
                                args.size,
                                args.dry_run,
                                &clob_client,
                                &signer,
                            ).await;
                        }
                    }

                    if position.is_active() {
                        let exit_price = match position.token_side {
                            Some(TokenSide::Yes) => best_bid_price(&dual_snapshot.yes).unwrap_or(0.0),
                            Some(TokenSide::No) => best_bid_price(&dual_snapshot.no).unwrap_or(0.0),
                            _ => 0.0,
                        };
                        
                        if exit_price > 0.0 && epoch_seconds % 30 == 0 {
                            let pnl_pct = (exit_price - position.entry_price) / position.entry_price * 100.0;
                            let pnl_usd = pnl_pct / 100.0 * args.size;
                            println!(
                                "[POSITION] {:?} entry={:.4} current={:.4} | PnL: {:.2}% (${:.2})",
                                position.token_side.unwrap(), position.entry_price, exit_price, pnl_pct, pnl_usd
                            );
                        }
                    }
                }
            }
        }
    }
    input_source.shutdown().await;
    Ok(())
}

async fn run_backtest(args: BacktestArgs) -> Result<()> {
    pipeline::run_backtest(args).await
}

fn run_monte_carlo(args: MonteCarloArgs) -> Result<()> {
    pipeline::run_monte_carlo(args)
}

fn run_parameter_sweep(args: SweepArgs) -> Result<()> {
    pipeline::run_parameter_sweep(args)
}

async fn run_fetch_pmxt(args: FetchPmxtArgs) -> Result<()> {
    pipeline::run_fetch_pmxt(args).await
}

async fn run_extract_midpoints(args: ExtractMidpointsArgs) -> Result<()> {
    pipeline::run_extract_midpoints(args).await
}

fn run_inspect_parquet(args: InspectParquetArgs) -> Result<()> {
    pipeline::run_inspect_parquet(args)
}

async fn run_list_markets(args: ListMarketsArgs) -> Result<()> {
    pipeline::run_list_markets(args).await
}

async fn run_backtest_pipeline(args: BacktestPipelineArgs) -> Result<()> {
    pipeline::run_backtest_pipeline(args).await
}

// ============================================================================
// Research CLI Implementation
// ============================================================================

async fn run_export_features(args: ExportFeaturesArgs) -> Result<()> {
    use crate::bot::research::{FeatureExporter, ResearchConfig};
    use std::path::PathBuf;

    println!("[EXPORT] Input: {}", args.input);
    println!("[EXPORT] Output: {}", args.out);

    let config = ResearchConfig::default();
    let exporter = FeatureExporter::new(config);

    let manifest_path = args.manifest_out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&args.out).with_extension("manifest.json"));

    let manifest_str = manifest_path.to_str().unwrap_or("");
    let (feature_count, label_count) = exporter
        .export(&args.input, &args.out, manifest_str)?;

    println!("[EXPORT] Wrote {} feature rows", feature_count);
    println!("[EXPORT] Wrote {} label rows", label_count);
    println!("[EXPORT] Manifest: {:?}", manifest_path);

    Ok(())
}

fn run_inspect_features(args: InspectFeaturesArgs) -> Result<()> {
    use crate::bot::research::feature_export::inspect_features;
    inspect_features(&args.input, args.sample)
}

async fn run_backtest_scores(args: BacktestScoresArgs) -> Result<()> {
    use crate::bot::research::{FusionMode, FusionEngine, FusionConfig};

    println!("[BACKTEST] Scores: {}", args.scores);
    println!("[BACKTEST] Strategy: {}", args.strategy);

    // Determine fusion mode
    let mode = match args.strategy.to_lowercase().as_str() {
        "heuristic" => FusionMode::HeuristicOnly,
        "qlib" => FusionMode::QlibOnly,
        "fused" => FusionMode::Fused,
        _ => anyhow::bail!("Invalid strategy mode: {}. Use heuristic, qlib, or fused.", args.strategy),
    };

    let config = FusionConfig::default();
    let _engine = FusionEngine::new(mode, config);

    println!("[BACKTEST] Mode: {:?}", mode);
    println!("[BACKTEST] Note: Full backtest requires feature data integration");

    Ok(())
}

async fn run_score_shadow(args: ScoreShadowArgs) -> Result<()> {
    use crate::bot::research::{FusionMode, FusionEngine, FusionConfig};

    println!("[SHADOW] Asset: {}", args.asset);
    println!("[SHADOW] Duration: {}", args.duration);
    println!("[SHADOW] Scores: {}", args.scores);
    println!("[SHADOW] Strategy: {}", args.strategy);

    let mode = match args.strategy.to_lowercase().as_str() {
        "heuristic" => FusionMode::HeuristicOnly,
        "qlib" => FusionMode::QlibOnly,
        "fused" => FusionMode::Fused,
        _ => anyhow::bail!("Invalid strategy mode: {}", args.strategy),
    };

    let config = FusionConfig::default();
    let _engine = FusionEngine::new(mode, config);

    println!("[SHADOW] Mode: {:?}", mode);
    println!("[SHADOW] Note: Shadow mode runs without live trading");
    println!("[SHADOW] Decision logging enabled");

    Ok(())
}

// Migrated to crate::bot::pipeline

#[cfg(test)]
mod tests {
    use crate::bot::feed::MarketSnapshot;
    use crate::bot::risk::{FilterReason, trade_allowed};
    use polymarket_client_sdk::types::Decimal;

    #[test]
    fn trade_allowed_passes_good_conditions() {
        let snapshot = MarketSnapshot {
            midpoint: Some(Decimal::new(50, 2)),
            best_bid: Some(Decimal::new(47, 2)),
            best_ask: Some(Decimal::new(50, 2)),
            spread: Some(Decimal::new(3, 2)),
            top5_bid_depth: Decimal::new(50000, 2),
            top5_ask_depth: Decimal::new(50000, 2),
        };
        // yes_ask=0.50, no_ask=0.50, sum=1.0, passes complement
        assert!(trade_allowed(&snapshot, 60, 30, 0.50, 0.50).is_ok());
    }

    #[test]
    fn trade_allowed_blocks_wide_spread() {
        let snapshot = MarketSnapshot {
            midpoint: Some(Decimal::new(50, 2)),
            best_bid: Some(Decimal::new(40, 2)),
            best_ask: Some(Decimal::new(60, 2)),
            spread: Some(Decimal::new(20, 2)),
            top5_bid_depth: Decimal::new(50000, 2),
            top5_ask_depth: Decimal::new(50000, 2),
        };
        assert_eq!(trade_allowed(&snapshot, 60, 30, 0.60, 0.40), Err(FilterReason::WideSpread));
    }

    #[test]
    fn trade_allowed_blocks_extreme_price() {
        let snapshot = MarketSnapshot {
            midpoint: Some(Decimal::new(85, 2)),
            best_bid: Some(Decimal::new(84, 2)),
            best_ask: Some(Decimal::new(86, 2)),
            spread: Some(Decimal::new(2, 2)),
            top5_bid_depth: Decimal::new(50000, 2),
            top5_ask_depth: Decimal::new(50000, 2),
        };
        assert_eq!(trade_allowed(&snapshot, 60, 30, 0.86, 0.14), Err(FilterReason::ExtremePrice));
    }

    #[test]
    fn trade_allowed_blocks_broken_book() {
        let snapshot = MarketSnapshot {
            midpoint: Some(Decimal::new(50, 2)),
            best_bid: Some(Decimal::new(49, 2)),
            best_ask: Some(Decimal::new(51, 2)),
            spread: Some(Decimal::new(2, 2)),
            top5_bid_depth: Decimal::new(50000, 2),
            top5_ask_depth: Decimal::new(50000, 2),
        };
        // YES=0.99, NO=0.99, sum=1.98 - broken book
        assert_eq!(trade_allowed(&snapshot, 60, 30, 0.99, 0.99), Err(FilterReason::BrokenBook));
    }
}



