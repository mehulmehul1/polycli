use crate::bot::candles::CandleEngine;
use crate::bot::feed::{DualSnapshot, MarketSnapshot, ReplayMode, ReplaySnapshotSource, StrategyInputSource};
use crate::bot::indicators::{IndicatorEngine, IndicatorState};
use crate::bot::logging::{EngineEvent, EngineEventLoggers};
use crate::bot::risk::{best_ask_price, best_bid_price, decimal_to_f64, midpoint_price, GatekeeperState};
use crate::bot::shadow::{ShadowPosition, ShadowStepResult, TokenSide};
use crate::bot::signal::{EntrySignal, SignalEngine};
use crate::bot::strategy_runner::run_shadow_strategy_step;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc, Timelike};
use clap::{Args, ValueEnum};
use polymarket_client_sdk::types::Decimal;
use serde::{Deserialize, Serialize};
use serde_json;
use crate::bot::backtest::{BacktestConfig, BacktestEngine, BeckerParser, PmxtFetcher};
use crate::bot::backtest::data::load_mock_data;
use crate::bot::backtest::metrics::ParameterSweepResult;
use crate::bot::monte_carlo::{MonteCarloSimulator, SimulationConfig};
use polymarket_client_sdk::{clob, gamma};
use polymarket_client_sdk::gamma::types::request::MarketBySlugRequest;
use polymarket_client_sdk::clob::types::response::MarketResponse;
use polymarket_client_sdk::gamma::types::response::Market;

// ── CLI Arg Structs ────────────────────────────────────────────────────────────

#[derive(Args, Clone)]
pub struct BacktestPipelineArgs {
    /// Optional local or remote parquet file to process directly
    #[arg(long)]
    pub input: Option<String>,

    /// Start time (ISO format: 2026-03-02T00:00:00Z)
    #[arg(long)]
    pub start: String,

    /// End time (ISO format: 2026-03-03T00:00:00Z)
    #[arg(long)]
    pub end: String,

    /// Market filter pattern (default: btc-updown-5m)
    #[arg(long, default_value = "btc-updown-5m")]
    pub filter: String,

    /// Use top N markets by tick count (skips Gamma API probing)
    #[arg(long)]
    pub top_n: Option<usize>,

    /// Minimum parquet ticks required before metadata lookup
    #[arg(long, default_value = "1")]
    pub min_ticks: usize,

    /// Crypto filter for exact market matching
    #[arg(long, value_enum, default_value_t = CryptoAsset::Btc)]
    pub crypto: CryptoAsset,

    /// Starting capital in USD
    #[arg(long, default_value = "100")]
    pub capital: f64,

    /// Position size in USD
    #[arg(long, default_value = "1")]
    pub size: f64,

    /// Entry band low
    #[arg(long, default_value = "0.35")]
    pub band_low: f64,

    /// Entry band high
    #[arg(long, default_value = "0.65")]
    pub band_high: f64,

    /// Export results to JSON
    #[arg(long)]
    pub export: Option<String>,

    /// Export tick data to CSV
    #[arg(long)]
    pub export_ticks: Option<String>,

    /// Replay mode for historical orderbook processing
    #[arg(long, value_enum, default_value_t = ReplayMode::EventByEvent)]
    pub replay_mode: ReplayMode,

    /// Optional JSONL event log path
    #[arg(long)]
    pub event_log: Option<String>,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Args, Clone)]
pub struct BacktestArgs {
    /// Path to Becker dataset (parquet or CSV). If not provided, uses mock data.
    #[arg(short, long)]
    pub data: Option<String>,

    /// Entry band low (default: 0.35)
    #[arg(long, default_value = "0.35")]
    pub band_low: f64,

    /// Entry band high (default: 0.65)
    #[arg(long, default_value = "0.65")]
    pub band_high: f64,

    /// Starting capital in USD
    #[arg(long, default_value = "100")]
    pub capital: f64,

    /// Position size in USD
    #[arg(long, default_value = "1")]
    pub size: f64,

    /// Export results to JSON file
    #[arg(long)]
    pub export: Option<String>,
}

#[derive(Args, Clone)]
pub struct MonteCarloArgs {
    /// Number of simulations to run
    #[arg(short = 'n', long, default_value = "10000")]
    pub simulations: usize,

    /// Number of trades per simulation
    #[arg(long, default_value = "500")]
    pub trades: usize,

    /// Position size as fraction of capital (e.g., 0.02 = 2%)
    #[arg(long, default_value = "0.02")]
    pub kelly: f64,

    /// Seed for reproducibility
    #[arg(long)]
    pub seed: Option<u64>,

    /// Import backtest results from JSON file
    #[arg(short, long)]
    pub input: Option<String>,

    /// Export results to JSON file
    #[arg(long)]
    pub export: Option<String>,
}

#[derive(Args, Clone)]
pub struct SweepArgs {
    /// Path to Becker dataset (optional, uses mock data if not provided)
    #[arg(short, long)]
    pub data: Option<String>,

    /// Export results to JSON file
    #[arg(long)]
    pub export: Option<String>,
}

#[derive(Args, Clone)]
pub struct FetchPmxtArgs {
    /// URL of the remote PMXT Parquet file
    #[arg(short, long)]
    pub url: String,

    /// Filter pattern for market slug (default: btc-updown-5m)
    #[arg(short, long, default_value = "btc-updown-5m")]
    pub filter: String,

    /// Export filtered data to CSV file
    #[arg(long)]
    pub csv: Option<String>,

    /// Export filtered data to Parquet file
    #[arg(long)]
    pub parquet: Option<String>,

    /// Run backtest on fetched data immediately
    #[arg(short, long)]
    pub backtest: bool,
}

#[derive(Args, Clone)]
pub struct ExtractMidpointsArgs {
    /// Path to local Parquet file (orderbook data)
    #[arg(short, long)]
    pub input: String,

    /// Filter pattern for market slug (default: btc-updown-5m)
    #[arg(short, long, default_value = "btc-updown-5m")]
    pub filter: String,

    /// Export to CSV file
    #[arg(long)]
    pub csv: Option<String>,

    /// Only fetch condition IDs from Gamma API (dry run)
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Clone)]
pub struct InspectParquetArgs {
    /// Path to local Parquet file
    #[arg(short, long)]
    pub input: String,

    /// Show sample rows (default: 10)
    #[arg(long, default_value = "10")]
    pub sample: usize,

    /// Filter by market_id (condition ID)
    #[arg(long)]
    pub market_id: Option<String>,

    /// Filter by token_id embedded in the data JSON payload
    #[arg(long, conflicts_with = "market_id")]
    pub token_id: Option<String>,
}

#[derive(Args, Clone)]
pub struct ListMarketsArgs {
    /// Path (or URL if using httpfs) to Parquet file
    #[arg(short, long)]
    pub input: String,

    /// Minimum ticks per market to include
    #[arg(short, long, default_value = "100")]
    pub min_ticks: usize,

    /// Output as table instead of JSON
    #[arg(short, long)]
    pub table: bool,

    /// Crypto asset filter
    #[arg(short, long, value_enum, default_value_t = CryptoAsset::Btc)]
    pub crypto: CryptoAsset,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BtStrategy {
    /// Scalper: EMA crossover + RSI + Bollinger Bands
    Scalper,
    /// Late window: high-prob sniping in final minutes
    LateWindow,
    /// Fair value: Logit jump-diffusion model for edge trading
    FairValue,
}

#[derive(Args, Clone)]
pub struct BacktestPmxtArgs {
    /// Directory containing PMXT parquet files
    #[arg(long)]
    pub input_dir: String,

    /// Strategy mode
    #[arg(long, value_enum, default_value_t = BtStrategy::Scalper)]
    pub strategy: BtStrategy,

    /// Starting capital in USD
    #[arg(long, default_value = "5")]
    pub capital: f64,

    /// Position size in USD per trade
    #[arg(long, default_value = "1")]
    pub size: f64,

    /// Entry band low (only for scalper)
    #[arg(long, default_value = "0.35")]
    pub band_low: f64,

    /// Entry band high (only for scalper)
    #[arg(long, default_value = "0.65")]
    pub band_high: f64,

    /// Minimum edge for late-window strategy
    #[arg(long, default_value = "0.05")]
    pub min_edge: f64,

    /// Filter pattern for market slug
    #[arg(long, default_value = "btc-updown-5m")]
    pub filter: String,

    /// Crypto asset filter
    #[arg(long, value_enum, default_value_t = CryptoAsset::Btc)]
    pub crypto: CryptoAsset,

    /// Minimum ticks per market
    #[arg(long, default_value = "1")]
    pub min_ticks: usize,

    /// Export results to JSON
    #[arg(long)]
    pub export: Option<String>,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

// ── Enums and Constants ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CryptoAsset {
    Btc,
    Eth,
    Sol,
    Xrp,
    All,
}

pub const BTC_UPDOWN_SLUG_PREFIX: &str = "btc-updown-5m-";
pub const FIVE_MINUTES_SECONDS: i64 = 300;

// ── Structs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredMarket {
    pub condition_id: String,
    pub slug: String,
    pub question: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub ticks: i64,
    pub min_ts: f64,
    pub max_ts: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GammaMarket {
    pub slug: Option<String>,
    #[serde(rename = "conditionId")]
    pub condition_id: Option<String>,
    #[serde(rename = "condition_id")]
    pub condition_id_alt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MidpointRow {
    pub market_id: String,
    pub market_slug: String,
    pub timestamp: i64,
    pub midpoint: f64,
    pub best_bid: f64,
    pub best_ask: f64,
}

#[derive(Debug, Clone)]
pub struct ParquetMarketStat {
    pub condition_id: String,
    pub ticks: i64,
    pub min_ts: f64,
    pub max_ts: f64,
}

#[derive(Debug, Clone)]
pub struct ResolvedSlugMarket {
    pub condition_id: String,
    pub slug: String,
    pub question: String,
    pub start_ts: i64,
    pub end_ts: i64,
}

#[derive(Debug, Serialize)]
pub struct ListMarketsOutput {
    pub input: String,
    pub crypto: CryptoAsset,
    pub min_ticks: usize,
    pub hour_start_ts: Option<i64>,
    pub discovered_count: usize,
    pub markets: Vec<DiscoveredMarket>,
}

#[derive(Debug, Clone)]
pub struct OrderbookTick {
    pub ts: f64,
    pub side: String,
    pub best_bid: f64,
    pub best_ask: f64,
}

#[derive(Debug, Clone)]
pub struct MarketState {
    pub condition_id: String,
    pub slug: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub ticks: Vec<OrderbookTick>,
}

#[derive(Debug, Serialize)]
pub struct PipelineMetrics {
    pub total_ticks: usize,
    pub total_markets: usize,
    pub trades_taken: usize,
    pub wins: usize,
    pub losses: usize,
    pub starting_capital: f64,
    pub ending_capital: f64,
    pub peak_capital: f64,
    pub max_drawdown: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
}

impl PipelineMetrics {
    pub fn new(starting_capital: f64) -> Self {
        Self {
            total_ticks: 0,
            total_markets: 0,
            trades_taken: 0,
            wins: 0,
            losses: 0,
            starting_capital,
            ending_capital: starting_capital,
            peak_capital: starting_capital,
            max_drawdown: 0.0,
            total_pnl: 0.0,
            total_pnl_pct: 0.0,
        }
    }

    pub fn print_summary(&self) {
        println!("\n================ PIPELINE SUMMARY ================");
        println!("Markets Processed: {}", self.total_markets);
        println!("Total Ticks:      {}", self.total_ticks);
        println!("Trades Taken:     {}", self.trades_taken);
        let win_rate = if self.trades_taken > 0 { self.wins as f64 / self.trades_taken as f64 * 100.0 } else { 0.0 };
        println!("Wins/Losses:      {} / {} ({:.1}%)", self.wins, self.losses, win_rate);
        println!("--------------------------------------------------");
        println!("Starting Capital: ${:.2}", self.starting_capital);
        println!("Ending Capital:   ${:.2}", self.ending_capital);
        println!("Total PnL:        ${:.2} ({:.2}%)", self.total_pnl, self.total_pnl_pct);
        println!("Max Drawdown:     {:.2}%", self.max_drawdown * 100.0);
        println!("==================================================");
    }
}

#[derive(Debug, Serialize)]
struct PipelineTickExport {
    market_slug: String,
    ts: u64,
    yes_bid: f64,
    yes_ask: f64,
    no_bid: f64,
    no_ask: f64,
}

#[derive(Debug, Clone)]
struct ReplayRow {
    ts: f64,
    side: String,
    bid: f64,
    ask: f64,
}

fn build_dual_snapshot(yes_bid: f64, yes_ask: f64, no_bid: f64, no_ask: f64, ts: f64) -> DualSnapshot {
    DualSnapshot {
        yes: MarketSnapshot {
            midpoint: Some(Decimal::from_f64_retain((yes_bid + yes_ask) / 2.0).unwrap_or_default()),
            best_bid: Some(Decimal::from_f64_retain(yes_bid).unwrap_or_default()),
            best_ask: Some(Decimal::from_f64_retain(yes_ask).unwrap_or_default()),
            spread: Some(Decimal::from_f64_retain(yes_ask - yes_bid).unwrap_or_default()),
            top5_bid_depth: Decimal::from(1000),
            top5_ask_depth: Decimal::from(1000),
        },
        no: MarketSnapshot {
            midpoint: Some(Decimal::from_f64_retain((no_bid + no_ask) / 2.0).unwrap_or_default()),
            best_bid: Some(Decimal::from_f64_retain(no_bid).unwrap_or_default()),
            best_ask: Some(Decimal::from_f64_retain(no_ask).unwrap_or_default()),
            spread: Some(Decimal::from_f64_retain(no_ask - no_bid).unwrap_or_default()),
            top5_bid_depth: Decimal::from(1000),
            top5_ask_depth: Decimal::from(1000),
        },
        ts_exchange: ts,
    }
}

fn build_replay_snapshots(rows: &[ReplayRow], replay_mode: ReplayMode) -> Vec<DualSnapshot> {
    let mut snapshots = Vec::new();
    let (mut yes_bid, mut yes_ask, mut no_bid, mut no_ask) = (0.0, 0.0, 0.0, 0.0);
    let mut last_complete_second: Option<u64> = None;

    match replay_mode {
        ReplayMode::EventByEvent => {
            let (mut prev_yes_bid, mut prev_yes_ask, mut prev_no_bid, mut prev_no_ask) = (0.0, 0.0, 0.0, 0.0);
            for row in rows {
                match row.side.as_str() {
                    "YES" => {
                        yes_bid = row.bid;
                        yes_ask = row.ask;
                    }
                    "NO" => {
                        no_bid = row.bid;
                        no_ask = row.ask;
                    }
                    _ => continue,
                }
                if yes_bid > 0.0 && yes_ask > 0.0 && no_bid > 0.0 && no_ask > 0.0 {
                    // Dedup: only emit snapshot when prices actually change
                    if (yes_bid - prev_yes_bid).abs() > 0.0001
                        || (yes_ask - prev_yes_ask).abs() > 0.0001
                        || (no_bid - prev_no_bid).abs() > 0.0001
                        || (no_ask - prev_no_ask).abs() > 0.0001
                    {
                        snapshots.push(build_dual_snapshot(yes_bid, yes_ask, no_bid, no_ask, row.ts));
                        prev_yes_bid = yes_bid;
                        prev_yes_ask = yes_ask;
                        prev_no_bid = no_bid;
                        prev_no_ask = no_ask;
                    }
                }
            }
        }
        ReplayMode::LiveParity1s => {
            for row in rows {
                let row_second = row.ts.floor() as u64;
                if let Some(previous_second) = last_complete_second {
                    if row_second > previous_second && yes_bid > 0.0 && yes_ask > 0.0 && no_bid > 0.0 && no_ask > 0.0 {
                        for second in previous_second..row_second {
                            snapshots.push(build_dual_snapshot(
                                yes_bid,
                                yes_ask,
                                no_bid,
                                no_ask,
                                second as f64,
                            ));
                        }
                    }
                }

                match row.side.as_str() {
                    "YES" => {
                        yes_bid = row.bid;
                        yes_ask = row.ask;
                    }
                    "NO" => {
                        no_bid = row.bid;
                        no_ask = row.ask;
                    }
                    _ => continue,
                }

                if yes_bid > 0.0 && yes_ask > 0.0 && no_bid > 0.0 && no_ask > 0.0 {
                    last_complete_second = Some(row_second);
                }
            }

            if let Some(last_second) = last_complete_second {
                snapshots.push(build_dual_snapshot(
                    yes_bid,
                    yes_ask,
                    no_bid,
                    no_ask,
                    last_second as f64,
                ));
            }
        }
    }

    snapshots
}

// ── Internal Helpers ───────────────────────────────────────────────────────────

pub fn generate_hourly_urls(start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<String> {
    let mut urls = Vec::new();
    let mut current = start;

    while current < end {
        let date_str = current.format("%Y-%m-%d").to_string();
        let hour = current.hour();
        let url = format!(
            "https://polymarket-archive.s3.us-east-2.amazonaws.com/orderbook/{}/orderbook_{:02}.parquet",
            date_str, hour
        );
        urls.push(url);
        current = current + chrono::Duration::hours(1);
    }
    urls
}

pub fn resolve_pipeline_inputs(
    input: Option<&str>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<String> {
    if let Some(path) = input {
        vec![path.to_string()]
    } else {
        generate_hourly_urls(start, end)
    }
}

pub fn update_pipeline_capital_metrics(metrics: &mut PipelineMetrics, bankroll: f64) {
    metrics.ending_capital = bankroll;
    if bankroll > metrics.peak_capital {
        metrics.peak_capital = bankroll;
    }
    if metrics.peak_capital > 0.0 {
        let drawdown = (metrics.peak_capital - bankroll) / metrics.peak_capital;
        if drawdown > metrics.max_drawdown {
            metrics.max_drawdown = drawdown;
        }
    }
}

pub fn extract_hour_from_filename(filename: &str) -> Option<i64> {
    let re = regex::Regex::new(r"(\d{4})-(\d{2})-(\d{2})T(\d{2})").ok()?;
    let caps = re.captures(filename)?;
    
    let year: i32 = caps.get(1)?.as_str().parse().ok()?;
    let month: u32 = caps.get(2)?.as_str().parse().ok()?;
    let day: u32 = caps.get(3)?.as_str().parse().ok()?;
    let hour: u32 = caps.get(4)?.as_str().parse().ok()?;
    
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_opt(hour, 0, 0))
        .map(|dt| dt.and_utc().timestamp())
}

pub async fn probe_btc_updown_slugs(hour_ts: i64) -> Result<Vec<(String, String)>> {
    let client = reqwest::Client::builder()
        .user_agent("polymarket-cli")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("Failed to build HTTP client")?;

    let mut result = Vec::new();
    let base_5m = hour_ts.div_euclid(FIVE_MINUTES_SECONDS) * FIVE_MINUTES_SECONDS;
    
    for offset in [-900, -600, -300, 0, 300, 600, 900, 1200, 1500, 1800, 2100, 2400, 2700, 3000, 3300, 3600] {
        let ts = base_5m + offset;
        let slug = format!("btc-updown-5m-{}", ts);
        let url = format!("https://gamma-api.polymarket.com/markets?slug={}", slug);
        
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<Vec<GammaMarket>>().await {
                    Ok(markets) => {
                        for m in markets {
                            if let (Some(s), Some(c)) = (m.slug.clone(), m.condition_id.or(m.condition_id_alt)) {
                                if !c.is_empty() {
                                    result.push((c, s));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(result)
}

pub async fn fetch_btc_condition_ids(filter: &str) -> Result<Vec<(String, String)>> {
    let client = reqwest::Client::builder()
        .user_agent("polymarket-cli")
        .build()
        .context("Failed to build HTTP client")?;

    let url = "https://gamma-api.polymarket.com/markets?limit=1000";
    let resp = client.get(url).send().await.context("Failed to call Gamma API")?;

    if !resp.status().is_success() {
        anyhow::bail!("Gamma API returned status: {}", resp.status());
    }

    let markets: Vec<GammaMarket> = resp.json().await.context("Failed to parse Gamma API response")?;
    let filter_lower = filter.to_lowercase();
    let mut result = Vec::new();

    for m in markets {
        let slug = match m.slug {
            Some(s) => s,
            None => continue,
        };
        if !slug.to_lowercase().contains(&filter_lower) {
            continue;
        }
        let cond_id = m.condition_id.or(m.condition_id_alt).unwrap_or_default();
        if !cond_id.is_empty() {
            result.push((cond_id, slug));
        }
    }
    Ok(result)
}

pub fn hour_btc_5m_slugs(hour_start_ts: i64) -> Vec<String> {
    (0..12)
        .map(|idx| {
            let start_ts = hour_start_ts + (idx as i64 * FIVE_MINUTES_SECONDS);
            format!("{BTC_UPDOWN_SLUG_PREFIX}{start_ts}")
        })
        .collect()
}

pub fn gamma_market_condition_id_hex(market: &Market) -> Option<String> {
    market
        .condition_id
        .as_ref()
        .map(|bytes| format!("0x{}", alloy::hex::encode(bytes.as_slice())))
}

async fn resolve_btc_hour_slug_markets(
    hour_start_ts: i64,
    verbose: bool,
) -> Result<Vec<ResolvedSlugMarket>> {
    let gamma_client = gamma::Client::default();
    let mut resolved = Vec::new();

    for slug in hour_btc_5m_slugs(hour_start_ts) {
        let req = MarketBySlugRequest::builder().slug(slug.clone()).build();
        let market = match gamma_client.market_by_slug(&req).await {
            Ok(market) => market,
            Err(err) => {
                if verbose {
                    eprintln!("[LIST] slug lookup failed for {}: {}", slug, err);
                }
                continue;
            }
        };

        if let Some(condition_id) = gamma_market_condition_id_hex(&market) {
            if let Some(start_ts) = parse_slug_timestamp(&slug) {
                resolved.push(ResolvedSlugMarket {
                    condition_id,
                    slug,
                    question: market.question.unwrap_or_default(),
                    start_ts,
                    end_ts: start_ts + FIVE_MINUTES_SECONDS,
                });
            }
        }
    }
    resolved.sort_by_key(|m| (m.start_ts, m.end_ts));
    resolved.dedup_by(|a, b| a.condition_id == b.condition_id);
    Ok(resolved)
}

pub fn is_updown_5m_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_updown = lower.contains("up") && lower.contains("down");
    let has_5m = lower.contains("5m") || lower.contains("5-minute") || lower.contains("5 minute");
    has_updown && has_5m
}

pub fn matches_crypto_text(text: &str, crypto: CryptoAsset) -> bool {
    let lower = text.to_ascii_lowercase();
    let has = |terms: &[&str]| terms.iter().any(|term| lower.contains(term));

    match crypto {
        CryptoAsset::Btc => has(&["btc", "bitcoin"]),
        CryptoAsset::Eth => has(&["eth", "ethereum"]),
        CryptoAsset::Sol => has(&["sol", "solana"]),
        CryptoAsset::Xrp => has(&["xrp", "ripple"]),
        CryptoAsset::All => has(&["btc", "bitcoin", "eth", "ethereum", "sol", "solana", "xrp", "ripple"]),
    }
}

pub fn has_binary_directional_tokens(market: &MarketResponse) -> bool {
    if market.tokens.len() != 2 {
        return false;
    }
    let mut has_positive = false;
    let mut has_negative = false;
    for token in &market.tokens {
        let outcome = token.outcome.to_ascii_lowercase();
        if matches!(outcome.as_str(), "yes" | "up" | "above") {
            has_positive = true;
        } else if matches!(outcome.as_str(), "no" | "down" | "below") {
            has_negative = true;
        }
    }
    has_positive && has_negative
}

pub fn parse_slug_timestamp(slug: &str) -> Option<i64> {
    let suffix = slug.rsplit('-').next()?;
    if suffix.len() < 10 || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<i64>().ok().filter(|ts| *ts > 0)
}

pub fn infer_market_window(market: &MarketResponse) -> Option<(i64, i64)> {
    if let Some(start_ts) = parse_slug_timestamp(&market.market_slug) {
        return Some((start_ts, start_ts + FIVE_MINUTES_SECONDS));
    }
    let end_ts = market.end_date_iso.as_ref()?.timestamp();
    let start_ts = end_ts - FIVE_MINUTES_SECONDS;
    Some((start_ts, end_ts))
}

pub fn window_overlaps_hour(start_ts: i64, end_ts: i64, hour_start_ts: i64) -> bool {
    let hour_end_ts = hour_start_ts + 3600;
    end_ts > hour_start_ts && start_ts < hour_end_ts
}

pub fn market_matches_exact_filter(
    market: &MarketResponse,
    crypto: CryptoAsset,
    slug_filter: Option<&str>,
) -> bool {
    let combined = format!(
        "{} {}",
        market.question.to_ascii_lowercase(),
        market.market_slug.to_ascii_lowercase()
    );

    if !is_updown_5m_text(&combined) {
        return false;
    }
    if !matches_crypto_text(&combined, crypto) {
        return false;
    }
    if !has_binary_directional_tokens(market) {
        return false;
    }

    if let Some(filter) = slug_filter {
        let normalized = filter.to_ascii_lowercase();
        if !normalized.is_empty() && !combined.contains(&normalized) {
            return false;
        }
    }
    true
}

fn load_parquet_market_stats(
    conn: &duckdb::Connection,
    input: &str,
    min_ticks: usize,
) -> Result<Vec<ParquetMarketStat>> {
    let sql = format!(
        r#"
        SELECT
            market_id,
            COUNT(*) AS ticks,
            MIN(CAST(data ->> '$.timestamp' AS DOUBLE)) AS min_ts,
            MAX(CAST(data ->> '$.timestamp' AS DOUBLE)) AS max_ts
        FROM read_parquet('{}')
        GROUP BY market_id
        HAVING COUNT(*) >= {}
        ORDER BY min_ts ASC
        "#,
        input, min_ticks
    );

    let mut stmt = conn.prepare(&sql).context("Failed to prepare market stats query")?;
    let rows = stmt.query_map([], |row| {
        Ok(ParquetMarketStat {
            condition_id: row.get::<_, String>(0)?,
            ticks: row.get::<_, i64>(1)?,
            min_ts: row.get::<_, f64>(2)?,
            max_ts: row.get::<_, f64>(3)?,
        })
    }).context("Failed to execute market stats query")?;

    let mut stats = Vec::new();
    for row in rows {
        stats.push(row?);
    }
    Ok(stats)
}

fn synthetic_slug_for_market(start_ts: i64, crypto: CryptoAsset) -> String {
    match crypto {
        CryptoAsset::Btc => format!("btc-updown-5m-{}", start_ts),
        CryptoAsset::Eth => format!("eth-updown-5m-{}", start_ts),
        CryptoAsset::Sol => format!("sol-updown-5m-{}", start_ts),
        CryptoAsset::Xrp => format!("xrp-updown-5m-{}", start_ts),
        CryptoAsset::All => format!("market-{}", start_ts),
    }
}

fn build_top_n_discovered_markets(
    stats: Vec<ParquetMarketStat>,
    top_n: usize,
    crypto: CryptoAsset,
    slug_filter: Option<&str>,
) -> Vec<DiscoveredMarket> {
    let normalized_filter = slug_filter
        .map(|filter| filter.trim().to_ascii_lowercase())
        .filter(|filter| !filter.is_empty());

    let mut discovered: Vec<DiscoveredMarket> = stats
        .into_iter()
        .filter_map(|stat| {
            let start_ts = (stat.min_ts.floor() as i64).div_euclid(FIVE_MINUTES_SECONDS) * FIVE_MINUTES_SECONDS;
            let end_ts = start_ts + FIVE_MINUTES_SECONDS;
            let slug = synthetic_slug_for_market(start_ts, crypto);

            if let Some(filter) = normalized_filter.as_deref() {
                if !slug.to_ascii_lowercase().contains(filter) {
                    return None;
                }
            }

            Some(DiscoveredMarket {
                question: format!("Synthetic replay market {}", stat.condition_id),
                condition_id: stat.condition_id,
                slug,
                start_ts,
                end_ts,
                ticks: stat.ticks,
                min_ts: stat.min_ts,
                max_ts: stat.max_ts,
            })
        })
        .collect();

    discovered.sort_by(|left, right| {
        right
            .ticks
            .cmp(&left.ticks)
            .then_with(|| left.start_ts.cmp(&right.start_ts))
            .then_with(|| left.condition_id.cmp(&right.condition_id))
    });
    discovered.truncate(top_n);
    discovered
}

fn load_parquet_market_stats_for_ids(
    conn: &duckdb::Connection,
    input: &str,
    condition_ids: &[String],
) -> Result<Vec<ParquetMarketStat>> {
    if condition_ids.is_empty() {
        return Ok(Vec::new());
    }
    let quoted_ids = condition_ids.iter().map(|id| format!("'{}'", id)).collect::<Vec<_>>().join(",");
    let sql = format!(
        r#"
        SELECT
            market_id,
            COUNT(*) AS ticks,
            MIN(CAST(data ->> '$.timestamp' AS DOUBLE)) AS min_ts,
            MAX(CAST(data ->> '$.timestamp' AS DOUBLE)) AS max_ts
        FROM read_parquet('{}')
        WHERE market_id IN ({})
        GROUP BY market_id
        ORDER BY min_ts ASC
        "#,
        input, quoted_ids
    );

    let mut stmt = conn.prepare(&sql).context("Failed to prepare filtered market stats query")?;
    let rows = stmt.query_map([], |row| {
        Ok(ParquetMarketStat {
            condition_id: row.get::<_, String>(0)?,
            ticks: row.get::<_, i64>(1)?,
            min_ts: row.get::<_, f64>(2)?,
            max_ts: row.get::<_, f64>(3)?,
        })
    }).context("Failed to execute filtered market stats query")?;

    let mut stats = Vec::new();
    for row in rows {
        stats.push(row?);
    }
    Ok(stats)
}

pub async fn discover_markets_for_input(
    conn: &duckdb::Connection,
    input: &str,
    min_ticks: usize,
    crypto: CryptoAsset,
    slug_filter: Option<&str>,
    verbose: bool,
) -> Result<Vec<DiscoveredMarket>> {
    let hour_start_ts = extract_hour_from_filename(input);

    if crypto == CryptoAsset::Btc {
        if let Some(hour_ts) = hour_start_ts {
            let slug_filter_allows_btc = slug_filter
                .map(|f| f.trim().is_empty() || f.to_ascii_lowercase().contains("btc-updown-5m"))
                .unwrap_or(true);

            if slug_filter_allows_btc {
                let resolved = resolve_btc_hour_slug_markets(hour_ts, verbose).await?;
                let resolved_ids: Vec<String> = resolved.iter().map(|m| m.condition_id.clone()).collect();
                let stats = load_parquet_market_stats_for_ids(conn, input, &resolved_ids)?;
                let stats_by_id: std::collections::HashMap<String, ParquetMarketStat> = stats
                    .into_iter()
                    .map(|s| (s.condition_id.clone(), s))
                    .collect();

                let mut discovered = Vec::new();
                for market in resolved {
                    if let Some(stat) = stats_by_id.get(&market.condition_id) {
                        if stat.ticks >= min_ticks as i64 {
                            discovered.push(DiscoveredMarket {
                                condition_id: market.condition_id,
                                slug: market.slug,
                                question: market.question,
                                start_ts: market.start_ts,
                                end_ts: market.end_ts,
                                ticks: stat.ticks,
                                min_ts: stat.min_ts,
                                max_ts: stat.max_ts,
                            });
                        }
                    }
                }
                discovered.sort_by_key(|m| (m.start_ts, m.end_ts));
                discovered.dedup_by(|a, b| a.condition_id == b.condition_id);
                return Ok(discovered);
            }
        }
    }

    let stats = load_parquet_market_stats(conn, input, min_ticks)?;
    let mut discovered = Vec::new();
    let total_candidates = stats.len();
    let mut completed = 0usize;
    let mut queue = std::collections::VecDeque::from(stats);
    let mut in_flight = tokio::task::JoinSet::new();
    let max_in_flight = 16usize;

    while !queue.is_empty() || !in_flight.is_empty() {
        while in_flight.len() < max_in_flight {
            let Some(stat) = queue.pop_front() else { break; };
            in_flight.spawn(async move {
                let client = clob::Client::default();
                let market = client.market(&stat.condition_id).await;
                (stat, market)
            });
        }

        let Some(join_result) = in_flight.join_next().await else { break; };
        completed += 1;
        if verbose && completed % 100 == 0 {
            eprintln!("[LIST] resolved metadata {}/{}", completed, total_candidates);
        }

        if let Ok((stat, Ok(market))) = join_result {
            if market_matches_exact_filter(&market, crypto, slug_filter) {
                if let Some((start_ts, end_ts)) = infer_market_window(&market) {
                    if end_ts > start_ts && start_ts % FIVE_MINUTES_SECONDS == 0 {
                        if hour_start_ts.is_none() || window_overlaps_hour(start_ts, end_ts, hour_start_ts.unwrap()) {
                            discovered.push(DiscoveredMarket {
                                condition_id: stat.condition_id,
                                slug: market.market_slug,
                                question: market.question,
                                start_ts,
                                end_ts,
                                ticks: stat.ticks,
                                min_ts: stat.min_ts,
                                max_ts: stat.max_ts,
                            });
                        }
                    }
                }
            }
        }
    }

    discovered.sort_by_key(|m| (m.start_ts, m.end_ts, m.condition_id.clone()));
    discovered.dedup_by(|a, b| a.condition_id == b.condition_id);
    Ok(discovered)
}

// ── Core Pipeline Functions ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn process_pipeline_snapshot(
    market: &DiscoveredMarket,
    dual_snapshot: &DualSnapshot,
    epoch_seconds: u64,
    candle_engine: &mut CandleEngine,
    ind_1m: &mut IndicatorEngine,
    ind_5s: &mut IndicatorEngine,
    signal_engine: &mut SignalEngine,
    state_1m: &mut IndicatorState,
    state_5s: &mut IndicatorState,
    shadow: &mut ShadowPosition,
    gatekeeper: &mut GatekeeperState,
    metrics: &mut PipelineMetrics,
    position_size_usd: f64,
    verbose: bool,
    event_loggers: Option<&EngineEventLoggers>,
) -> Option<ShadowStepResult> {
    metrics.total_ticks += 1;

    if let Some(loggers) = event_loggers {
        loggers.log_market(EngineEvent::BookUpdate {
            ts: epoch_seconds,
            market_slug: market.slug.clone(),
            source: "replay".to_string(),
            yes_bid: dual_snapshot.yes.best_bid.map(decimal_to_f64).unwrap_or(0.0),
            yes_ask: dual_snapshot.yes.best_ask.map(decimal_to_f64).unwrap_or(0.0),
            no_bid: dual_snapshot.no.best_bid.map(decimal_to_f64).unwrap_or(0.0),
            no_ask: dual_snapshot.no.best_ask.map(decimal_to_f64).unwrap_or(0.0),
        });
    }

    if let Some(step) = run_shadow_strategy_step(
        dual_snapshot,
        &market.slug,
        &market.slug,
        market.start_ts,
        market.end_ts,
        epoch_seconds,
        position_size_usd,
        candle_engine,
        ind_1m,
        ind_5s,
        signal_engine,
        state_1m,
        state_5s,
        shadow,
        gatekeeper,
        event_loggers,
    ) {
        if step.entry_blocked && verbose {
            println!("[PIPELINE] blocked entry {}", market.condition_id);
        }
        if let Some(exit_trade) = &step.exit_trade {
            metrics.trades_taken += 1;
            if exit_trade.pnl_usd >= 0.0 {
                metrics.wins += 1;
            } else {
                metrics.losses += 1;
            }
            update_pipeline_capital_metrics(metrics, shadow.bankroll_usd);
        }
        return Some(step);
    }
    None
}

use crate::bot::pricing::{
    CalibratedFairValue, FairValueModel, JumpCalibrator, KalmanFilter,
    LogitJumpDiffusion, LogitObservation, sigmoid, prob_to_logit, risk_neutral_drift,
};

/// Process a single snapshot using the full FairValue pipeline:
/// 1. Logit observation from market midpoint
/// 2. Kalman filter for noise reduction
/// 3. Jump calibrator (EM) for jump parameter estimation
/// 4. Horizon classification for parameter adaptation
/// 5. Fair probability via risk-neutral model
/// 6. Calibrated edge with spread/book adjustment
/// 7. Momentum gate (velocity + EMA alignment)
/// 8. Entry/exit decisions
pub fn process_fairvalue_snapshot(
    logit_model: &mut LogitJumpDiffusion,
    kalman: &mut Option<KalmanFilter>,
    jump_calibrator: &mut Option<JumpCalibrator>,
    calibrated_fv: &mut CalibratedFairValue,
    snapshot: &DualSnapshot,
    epoch_seconds: u64,
    market_start_ts: i64,
    market_end_ts: i64,
    shadow: &mut ShadowPosition,
    metrics: &mut PipelineMetrics,
    position_size_usd: f64,
    min_edge: f64,
    verbose: bool,
    cumulative_wins: &mut Vec<f64>,
    cumulative_losses: &mut Vec<f64>,
) {
    let yes_mid = midpoint_price(&snapshot.yes).unwrap_or(0.5);
    let no_mid = 1.0 - yes_mid;
    let time_remaining = (market_end_ts - epoch_seconds as i64).max(0);

    if time_remaining <= 0 {
        return;
    }

    let yes_bid = best_bid_price(&snapshot.yes).unwrap_or(0.0);
    let yes_ask = best_ask_price(&snapshot.yes).unwrap_or(1.0);
    let no_bid = best_bid_price(&snapshot.no).unwrap_or(0.0);
    let no_ask = best_ask_price(&snapshot.no).unwrap_or(1.0);
    let spread = (yes_ask - yes_bid).max(0.001);
    let book_sum = yes_ask + no_ask;

    // Step 1: Create logit observation
    let clamped_prob = yes_mid.clamp(0.01, 0.99);
    let raw_logit = prob_to_logit(clamped_prob);

    let obs = LogitObservation {
        timestamp: epoch_seconds as i64,
        prob: clamped_prob,
        logit: raw_logit,
        spread,
        volume: 0.0,
    };

    // Step 2: Update logit model (exponential filter)
    let filtered = logit_model.update(&obs);

    // Step 3: Kalman filter for noise reduction
    let kalman_logit = if let Some(kf) = kalman {
        kf.set_measurement_noise(spread);
        let dt = 1.0 / (365.25 * 24.0 * 3600.0); // ~1 second in years
        let drift = risk_neutral_drift(filtered.logit, filtered.vol, 0.0);
        kf.predict(dt, drift, filtered.vol);
        kf.update(raw_logit, spread);
        kf.state()
    } else {
        filtered.logit
    };

    // Step 4: Update jump calibrator (EM estimation)
    if let Some(jc) = jump_calibrator {
        jc.update(&obs);
    }

    // Step 5: Horizon classification
    let horizon = crate::bot::market_classifier::classify_market(time_remaining);
    let edge_multiplier = horizon.edge_multiplier();

    // Step 6: Compute fair probability via risk-neutral model
    let fair = logit_model.fair_prob(time_remaining);
    let fair_prob = fair.expected;

    // Step 7: Edge calculation (use raw fair probability + spread adjustment)
    let spread_cost = (yes_ask - yes_bid) * 0.5;  // half spread as cost
    let book_inefficiency = 1.0 - book_sum;  // book_sum < 1.0 means cheap
    let adjusted_fair = (fair_prob - spread_cost + book_inefficiency * 0.1).clamp(0.01, 0.99);

    let edge_yes = (adjusted_fair - yes_ask) * edge_multiplier;
    let edge_no = ((1.0 - adjusted_fair) - no_ask) * edge_multiplier;

    // Step 8: Momentum gate (EMA alignment from indicators)
    // Use the filtered logit converted back to probability for EMA comparison
    let filtered_prob = sigmoid(kalman_logit);
    let ema_fast = filtered_prob;  // Use filtered probability as "fast EMA"
    let ema_slow = clamped_prob;   // Raw market midpoint as "slow EMA"
    let velocity = filtered_prob - clamped_prob;  // Direction of movement

    let momentum_allows_yes = velocity > -0.002 && ema_fast >= (ema_slow - 0.002);
    let momentum_allows_no = velocity < 0.002 && ema_fast <= (ema_slow + 0.002);

    // Step 9: Jump detection
    let is_jump = if let Some(jc) = jump_calibrator {
        let dt = 1.0 / (365.25 * 24.0 * 3600.0);
        jc.is_jump(kalman_logit, dt, 2.0)
    } else {
        false
    };

    // Trading cost
    let trading_cost = 0.025;

    // ── Entry Logic ──
    if !shadow.is_active() {
        let effective_min_edge = min_edge * edge_multiplier;

        // YES entry: calibrated edge + momentum + no extreme price + no jump
        if edge_yes > effective_min_edge
            && momentum_allows_yes
            && !is_jump
            && yes_ask < 0.95
            && yes_ask > 0.05
            && time_remaining > 30
        {
            shadow.token_side = Some(TokenSide::Yes);
            shadow.active_entry = Some(EntrySignal::Long);
            shadow.entry_price = yes_ask;
            shadow.size = 1.0;
            shadow.position_size_usd = position_size_usd;
            shadow.bankroll_usd -= position_size_usd;
            shadow.entry_timestamp = epoch_seconds;
            if verbose {
                println!("[FV ENTRY] YES @ {:.4} | Fair: {:.4} | Adj: {:.4} | Edge: {:.4}x{:.1} | Horizon: {} | Jump: {} | Bankroll: ${:.2}",
                    yes_ask, fair_prob, adjusted_fair, edge_yes, edge_multiplier, horizon.name(), is_jump, shadow.bankroll_usd);
            }
        }
        // NO entry
        else if edge_no > effective_min_edge
            && momentum_allows_no
            && !is_jump
            && no_ask < 0.95
            && no_ask > 0.05
            && time_remaining > 30
        {
            shadow.token_side = Some(TokenSide::No);
            shadow.active_entry = Some(EntrySignal::Short);
            shadow.entry_price = no_ask;
            shadow.size = 1.0;
            shadow.position_size_usd = position_size_usd;
            shadow.bankroll_usd -= position_size_usd;
            shadow.entry_timestamp = epoch_seconds;
            if verbose {
                println!("[FV ENTRY] NO @ {:.4} | Fair(YES): {:.4} | Adj: {:.4} | Edge: {:.4}x{:.1} | Horizon: {} | Jump: {} | Bankroll: ${:.2}",
                    no_ask, fair_prob, adjusted_fair, edge_no, edge_multiplier, horizon.name(), is_jump, shadow.bankroll_usd);
            }
        }
    }

    // ── Exit Logic ──
    if shadow.is_active() {
        let exit_price = match shadow.token_side {
            Some(TokenSide::Yes) => best_bid_price(&snapshot.yes),
            Some(TokenSide::No) => best_bid_price(&snapshot.no),
            _ => None,
        };

        let should_exit = if let Some(price) = exit_price {
            let current_edge = match shadow.token_side {
                Some(TokenSide::Yes) => adjusted_fair - price - trading_cost,
                Some(TokenSide::No) => (1.0 - adjusted_fair) - price - trading_cost,
                _ => 0.0,
            };

            // Exit conditions:
            // 1. Edge reversed (model now disagrees with position)
            let edge_reversed = current_edge < 0.0;
            // 2. Extreme price (near resolution)
            let extreme_price = price > 0.95 || price < 0.05;
            // 3. Time almost up
            let time_up = time_remaining < 30;
            // 4. Jump detected while holding (adverse event)
            let jump_exit = is_jump && current_edge < 0.02;

            edge_reversed || extreme_price || time_up || jump_exit
        } else {
            false
        };

        if should_exit {
            if let Some(price) = exit_price {
                let pnl = shadow.pnl(price);
                let dollar_pnl = pnl * shadow.position_size_usd;
                shadow.bankroll_usd += shadow.position_size_usd + dollar_pnl;
                metrics.trades_taken += 1;
                if dollar_pnl >= 0.0 {
                    metrics.wins += 1;
                    cumulative_wins.push(dollar_pnl);
                } else {
                    metrics.losses += 1;
                    cumulative_losses.push(dollar_pnl);
                }
                if verbose {
                    let exit_edge = match shadow.token_side {
                        Some(TokenSide::Yes) => adjusted_fair - price - trading_cost,
                        Some(TokenSide::No) => (1.0 - adjusted_fair) - price - trading_cost,
                        _ => 0.0,
                    };
                    println!("[FV EXIT] {:.4}% | ${:.4} | Edge: {:.4} | Bankroll: ${:.2}",
                        pnl * 100.0, dollar_pnl, exit_edge, shadow.bankroll_usd);
                }
                shadow.reset(epoch_seconds);
            }
        }
    }
}

pub async fn run_backtest_pipeline(args: BacktestPipelineArgs) -> Result<()> {
    let start: DateTime<Utc> = args.start.parse().context("Invalid start time format")?;
    let end: DateTime<Utc> = args.end.parse().context("Invalid end time format")?;
    let event_loggers = args
        .event_log
        .as_deref()
        .map(EngineEventLoggers::new)
        .transpose()
        .context("Failed to create pipeline event logs")?;

    println!("[PIPELINE] Backtest Pipeline: {} to {}", start, end);
    let inputs = resolve_pipeline_inputs(args.input.as_deref(), start, end);
    
    let conn = duckdb::Connection::open_in_memory()?;
    conn.execute_batch("INSTALL httpfs; LOAD httpfs; PRAGMA threads=4;")?;

    let mut metrics = PipelineMetrics::new(args.capital);
    let mut shadow = ShadowPosition::default();
    shadow.bankroll_usd = args.capital;
    let mut processed_markets = std::collections::HashSet::new();
    let mut exported_ticks = if args.export_ticks.is_some() {
        Some(Vec::new())
    } else {
        None
    };

    let bar = indicatif::ProgressBar::new(inputs.len() as u64);
    for input in &inputs {
        let discovered = if let Some(top_n) = args.top_n {
            let stats = load_parquet_market_stats(&conn, input, args.min_ticks)?;
            build_top_n_discovered_markets(stats, top_n, args.crypto, Some(&args.filter))
        } else {
            discover_markets_for_input(&conn, input, args.min_ticks, args.crypto, Some(&args.filter), args.verbose).await?
        };
        for market in discovered {
            if market.end_ts < start.timestamp() || market.start_ts > end.timestamp() { continue; }
            if !processed_markets.insert(market.condition_id.clone()) { continue; }

            metrics.total_markets += 1;
            let mut candle_engine = CandleEngine::new();
            let mut ind_1m = IndicatorEngine::new();
            let mut ind_5s = IndicatorEngine::new();
            let mut signal_engine = SignalEngine::new_with_band(args.band_low, args.band_high);
            let mut state_1m = IndicatorState::default();
            let mut state_5s = IndicatorState::default();
            let mut gatekeeper = GatekeeperState::new(args.size * 3.0, 15);
            shadow.full_reset();

            let tick_sql = format!(
                "SELECT COALESCE(TRY_CAST(data->>'$.timestamp' AS DOUBLE), 0.0) as ts, COALESCE(UPPER(data->>'$.side'), '') as side, COALESCE(TRY_CAST(data->>'$.best_bid' AS DOUBLE), 0.0) as bid, COALESCE(TRY_CAST(data->>'$.best_ask' AS DOUBLE), 0.0) as ask FROM read_parquet('{}') WHERE market_id = '{}' ORDER BY ts",
                input, market.condition_id
            );
            let mut stmt = conn.prepare(&tick_sql)?;
            let rows = stmt.query_map([], |row| {
                Ok(ReplayRow {
                    ts: row.get::<_, f64>(0)?,
                    side: row.get::<_, String>(1)?,
                    bid: row.get::<_, f64>(2)?,
                    ask: row.get::<_, f64>(3)?,
                })
            })?;

            let mut replay_rows = Vec::new();
            for row in rows {
                replay_rows.push(row?);
            }
            let snapshots = build_replay_snapshots(&replay_rows, args.replay_mode);
            let mut replay_source = ReplaySnapshotSource::new(snapshots);

            let mut last_yes_bid = 0.0;
            let mut last_no_bid = 0.0;
            while let Some(snapshot) = replay_source.next_snapshot().await? {
                last_yes_bid = snapshot.yes.best_bid.map(decimal_to_f64).unwrap_or(last_yes_bid);
                last_no_bid = snapshot.no.best_bid.map(decimal_to_f64).unwrap_or(last_no_bid);
                let epoch_seconds = replay_source
                    .current_time()
                    .unwrap_or_else(|| snapshot.ts_exchange.floor() as u64);
                if let Some(rows) = &mut exported_ticks {
                    rows.push(PipelineTickExport {
                        market_slug: market.slug.clone(),
                        ts: epoch_seconds,
                        yes_bid: snapshot.yes.best_bid.map(decimal_to_f64).unwrap_or(0.0),
                        yes_ask: snapshot.yes.best_ask.map(decimal_to_f64).unwrap_or(0.0),
                        no_bid: snapshot.no.best_bid.map(decimal_to_f64).unwrap_or(0.0),
                        no_ask: snapshot.no.best_ask.map(decimal_to_f64).unwrap_or(0.0),
                    });
                }
                process_pipeline_snapshot(
                    &market,
                    &snapshot,
                    epoch_seconds,
                    &mut candle_engine,
                    &mut ind_1m,
                    &mut ind_5s,
                    &mut signal_engine,
                    &mut state_1m,
                    &mut state_5s,
                    &mut shadow,
                    &mut gatekeeper,
                    &mut metrics,
                    args.size,
                    args.verbose,
                    event_loggers.as_ref(),
                );
            }

            if shadow.is_active() {
                let side_bid = if shadow.token_side == Some(TokenSide::Yes) {
                    last_yes_bid
                } else {
                    last_no_bid
                };
                let settlement = if side_bid > 0.5 { 1.0 } else { 0.0 };
                let pnl = shadow.pnl(settlement) * shadow.position_size_usd;
                shadow.bankroll_usd += shadow.position_size_usd + pnl;
                metrics.trades_taken += 1;
                if pnl >= 0.0 { metrics.wins += 1; } else { metrics.losses += 1; }
                shadow.reset(market.end_ts as u64);
            }
        }
        bar.inc(1);
    }
    bar.finish();
    metrics.ending_capital = shadow.bankroll_usd;
    metrics.total_pnl = metrics.ending_capital - metrics.starting_capital;
    metrics.total_pnl_pct = (metrics.total_pnl / metrics.starting_capital) * 100.0;
    if let Some(path) = args.export {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, &metrics)?;
    }
    if let Some(path) = args.export_ticks {
        let mut writer = csv::Writer::from_path(path)?;
        if let Some(rows) = exported_ticks {
            for row in rows {
                writer.serialize(row)?;
            }
        }
        writer.flush()?;
    }
    metrics.print_summary();
    Ok(())
}

pub async fn run_backtest_pmxt(args: BacktestPmxtArgs) -> Result<()> {
    use std::path::Path;
    use std::collections::HashSet;

    // Collect all parquet files from directory
    let dir = Path::new(&args.input_dir);
    if !dir.is_dir() {
        anyhow::bail!("Not a directory: {}", args.input_dir);
    }

    let mut parquet_files: Vec<(String, i64)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("parquet") {
            if let Some(ts) = extract_hour_from_filename(&path.to_string_lossy()) {
                parquet_files.push((path.to_string_lossy().to_string(), ts));
            }
        }
    }

    if parquet_files.is_empty() {
        anyhow::bail!("No parquet files found in {}", args.input_dir);
    }

    // Sort by timestamp
    parquet_files.sort_by_key(|(_, ts)| *ts);
    println!("[BACKTEST-PMXT] Found {} parquet files (sorted chronologically)", parquet_files.len());
    println!("[BACKTEST-PMXT] Strategy: {:?}", args.strategy);
    println!("[BACKTEST-PMXT] Starting capital: ${:.2}", args.capital);

    // Strategy config
    let (band_low, band_high) = match args.strategy {
        BtStrategy::Scalper => (args.band_low, args.band_high),
        BtStrategy::LateWindow => (0.85, 0.98),
        BtStrategy::FairValue => (0.05, 0.95), // wide band, model filters instead
    };

    // Setup DuckDB
    let conn = duckdb::Connection::open_in_memory()?;
    conn.execute_batch("INSTALL httpfs; LOAD httpfs; PRAGMA threads=4;")?;

    // Global state (persists across files)
    let mut metrics = PipelineMetrics::new(args.capital);
    let mut shadow = ShadowPosition::default();
    shadow.bankroll_usd = args.capital;
    let mut processed_markets: HashSet<String> = HashSet::new();
    let mut file_num = 0;
    let mut cumulative_wins: Vec<f64> = Vec::new();
    let mut cumulative_losses: Vec<f64> = Vec::new();
    let mut bankroll_history: Vec<(String, f64)> = Vec::new(); // (slug, bankroll after market)

    for (file_path, file_ts) in &parquet_files {
        file_num += 1;
        let hour_label = chrono::DateTime::from_timestamp(*file_ts, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| format!("ts={}", file_ts));

        println!("\n[FILE {}/{}] {} — {}", file_num, parquet_files.len(), hour_label, file_path);

        // Discover markets in this file (uses Gamma API to verify BTC 5m)
        let discovered = match discover_markets_for_input(&conn, file_path, args.min_ticks, args.crypto, Some(&args.filter), args.verbose).await {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  [WARN] Discovery failed for {}: {}", file_path, e);
                continue;
            }
        };

        if discovered.is_empty() {
            println!("  No markets found in this file");
            continue;
        }

        // Sort markets by start time
        let mut sorted_markets = discovered;
        sorted_markets.sort_by_key(|m| m.start_ts);

        for market in sorted_markets {
            if !processed_markets.insert(market.condition_id.clone()) {
                continue;
            }

            metrics.total_markets += 1;

            // Fresh indicator/signal state per market
            let mut candle_engine = CandleEngine::new();
            let mut ind_1m = IndicatorEngine::new();
            let mut ind_5s = IndicatorEngine::new();
            let mut signal_engine = SignalEngine::new_with_band(band_low, band_high);
            let mut state_1m = IndicatorState::default();
            let mut state_5s = IndicatorState::default();
            let mut gatekeeper = GatekeeperState::new(args.size * 3.0, 15);
            shadow.full_reset();
            let mut fv_model = LogitJumpDiffusion::new();
            let mut fv_kalman = Some(KalmanFilter::new(0.0));
            let mut fv_jump_calibrator = Some(JumpCalibrator::with_defaults());
            let mut fv_calibrated = {
                let base_model = FairValueModel::default();
                CalibratedFairValue::with_defaults(base_model)
            };

            // Query ticks for this market
            let tick_sql = format!(
                "SELECT COALESCE(TRY_CAST(data->>'$.timestamp' AS DOUBLE), 0.0) as ts, \
                        COALESCE(UPPER(data->>'$.side'), '') as side, \
                        COALESCE(TRY_CAST(data->>'$.best_bid' AS DOUBLE), 0.0) as bid, \
                        COALESCE(TRY_CAST(data->>'$.best_ask' AS DOUBLE), 0.0) as ask \
                 FROM read_parquet('{}') WHERE market_id = '{}' ORDER BY ts",
                file_path, market.condition_id
            );

            let mut stmt = match conn.prepare(&tick_sql) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  [WARN] SQL failed for {}: {}", market.slug, e);
                    continue;
                }
            };

            let rows = match stmt.query_map([], |row| {
                Ok(ReplayRow {
                    ts: row.get::<_, f64>(0)?,
                    side: row.get::<_, String>(1)?,
                    bid: row.get::<_, f64>(2)?,
                    ask: row.get::<_, f64>(3)?,
                })
            }) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("  [WARN] Query failed for {}: {}", market.slug, e);
                    continue;
                }
            };

            let mut replay_rows = Vec::new();
            for row in rows {
                replay_rows.push(row?);
            }

            if replay_rows.is_empty() {
                continue;
            }

            let snapshots = build_replay_snapshots(&replay_rows, ReplayMode::EventByEvent);
            let mut replay_source = ReplaySnapshotSource::new(snapshots);

            let mut last_yes_bid = 0.0;
            let mut last_no_bid = 0.0;

            while let Some(snapshot) = replay_source.next_snapshot().await? {
                last_yes_bid = snapshot.yes.best_bid.map(decimal_to_f64).unwrap_or(last_yes_bid);
                last_no_bid = snapshot.no.best_bid.map(decimal_to_f64).unwrap_or(last_no_bid);
                let epoch_seconds = replay_source
                    .current_time()
                    .unwrap_or_else(|| snapshot.ts_exchange.floor() as u64);

                if args.strategy == BtStrategy::FairValue {
                    // Fair value strategy uses full logit pipeline
                    process_fairvalue_snapshot(
                        &mut fv_model,
                        &mut fv_kalman,
                        &mut fv_jump_calibrator,
                        &mut fv_calibrated,
                        &snapshot,
                        epoch_seconds,
                        market.start_ts,
                        market.end_ts,
                        &mut shadow,
                        &mut metrics,
                        args.size,
                        args.min_edge,
                        args.verbose,
                        &mut cumulative_wins,
                        &mut cumulative_losses,
                    );
                } else {
                    // Standard strategy (scalper/late-window)
                    if let Some(step) = process_pipeline_snapshot(
                    &market,
                    &snapshot,
                    epoch_seconds,
                    &mut candle_engine,
                    &mut ind_1m,
                    &mut ind_5s,
                    &mut signal_engine,
                    &mut state_1m,
                    &mut state_5s,
                    &mut shadow,
                    &mut gatekeeper,
                    &mut metrics,
                    args.size,
                    args.verbose,
                    None,
                ) {
                    if let Some(exit_trade) = step.exit_trade {
                        if exit_trade.pnl_usd >= 0.0 {
                            cumulative_wins.push(exit_trade.pnl_usd);
                        } else {
                            cumulative_losses.push(exit_trade.pnl_usd);
                        }
                    }
                }
                } // end else (non-fairvalue strategy)
            }

            // Settle any open position at end of market
            if shadow.is_active() {
                let side_bid = if shadow.token_side == Some(TokenSide::Yes) {
                    last_yes_bid
                } else {
                    last_no_bid
                };
                // Binary resolution: if the held side's last bid > 0.5, it won (1.0), else lost (0.0)
                let settlement = if side_bid > 0.5 { 1.0 } else { 0.0 };
                let pnl = shadow.pnl(settlement) * shadow.position_size_usd;
                shadow.bankroll_usd += shadow.position_size_usd + pnl;
                metrics.trades_taken += 1;
                if pnl >= 0.0 {
                    metrics.wins += 1;
                    cumulative_wins.push(pnl);
                } else {
                    metrics.losses += 1;
                    cumulative_losses.push(pnl);
                }
                shadow.reset(market.end_ts as u64);
            }

            // Track bankroll
            bankroll_history.push((market.slug.clone(), shadow.bankroll_usd));

            // Update peak/drawdown
            if shadow.bankroll_usd > metrics.peak_capital {
                metrics.peak_capital = shadow.bankroll_usd;
            }
            let drawdown = (metrics.peak_capital - shadow.bankroll_usd) / metrics.peak_capital;
            if drawdown > metrics.max_drawdown {
                metrics.max_drawdown = drawdown;
            }

            if args.verbose {
                println!("  {} | Bankroll: ${:.2} | Ticks: {}", market.slug, shadow.bankroll_usd, market.ticks);
            }
        }
    }

    // Final metrics
    metrics.ending_capital = shadow.bankroll_usd;
    metrics.total_pnl = metrics.ending_capital - metrics.starting_capital;
    metrics.total_pnl_pct = (metrics.total_pnl / metrics.starting_capital) * 100.0;

    // Print enhanced summary
    println!("\n================ BACKTEST-PMXT SUMMARY ================");
    println!("Files Processed:    {}", parquet_files.len());
    println!("Markets Processed:  {}", metrics.total_markets);
    println!("Total Ticks:        {}", metrics.total_ticks);
    println!("Trades Taken:       {}", metrics.trades_taken);
    let win_rate = if metrics.trades_taken > 0 {
        metrics.wins as f64 / metrics.trades_taken as f64 * 100.0
    } else { 0.0 };
    println!("Wins/Losses:        {} / {} ({:.1}%)", metrics.wins, metrics.losses, win_rate);

    let avg_win = if !cumulative_wins.is_empty() {
        cumulative_wins.iter().sum::<f64>() / cumulative_wins.len() as f64
    } else { 0.0 };
    let avg_loss = if !cumulative_losses.is_empty() {
        cumulative_losses.iter().sum::<f64>() / cumulative_losses.len() as f64
    } else { 0.0 };
    let profit_factor = if avg_loss.abs() > 0.0001 {
        avg_win / avg_loss.abs()
    } else if avg_win > 0.0 { f64::INFINITY } else { 0.0 };

    println!("Avg Win:            ${:.4}", avg_win);
    println!("Avg Loss:           ${:.4}", avg_loss);
    println!("Profit Factor:      {:.2}", profit_factor);
    println!("------------------------------------------------------");
    println!("Starting Capital:   ${:.2}", metrics.starting_capital);
    println!("Ending Capital:     ${:.2}", metrics.ending_capital);
    println!("Total PnL:          ${:.2} ({:.2}%)", metrics.total_pnl, metrics.total_pnl_pct);
    println!("Max Drawdown:       {:.2}%", metrics.max_drawdown * 100.0);
    println!("======================================================");

    if let Some(path) = &args.export {
        #[derive(Serialize)]
        struct ExportResult {
            files_processed: usize,
            markets_processed: usize,
            total_ticks: usize,
            trades_taken: usize,
            wins: usize,
            losses: usize,
            win_rate: f64,
            avg_win: f64,
            avg_loss: f64,
            profit_factor: f64,
            starting_capital: f64,
            ending_capital: f64,
            total_pnl: f64,
            total_pnl_pct: f64,
            max_drawdown: f64,
            bankroll_history: Vec<(String, f64)>,
        }
        let result = ExportResult {
            files_processed: parquet_files.len(),
            markets_processed: metrics.total_markets,
            total_ticks: metrics.total_ticks,
            trades_taken: metrics.trades_taken,
            wins: metrics.wins,
            losses: metrics.losses,
            win_rate,
            avg_win,
            avg_loss,
            profit_factor,
            starting_capital: metrics.starting_capital,
            ending_capital: metrics.ending_capital,
            total_pnl: metrics.total_pnl,
            total_pnl_pct: metrics.total_pnl_pct,
            max_drawdown: metrics.max_drawdown,
            bankroll_history,
        };
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, &result)?;
        println!("[EXPORT] Results written to {}", path);
    }

    Ok(())
}

pub async fn run_backtest(args: BacktestArgs) -> Result<()> {
    let markets = if let Some(path) = &args.data {
        let mut parser = BeckerParser::new();
        if path.ends_with(".parquet") { parser.load_parquet(path)?; } else { parser.load_csv(path)?; }
        parser.organize_by_market();
        parser.get_markets().values().cloned().collect()
    } else {
        load_mock_data()
    };

    let metrics = BacktestEngine::new(BacktestConfig {
        entry_band_low: args.band_low,
        entry_band_high: args.band_high,
        position_size_usd: args.size,
        starting_capital: args.capital,
        ..Default::default()
    }).run_all(&markets);
    metrics.print_summary();
    if let Some(path) = args.export { metrics.export_json(&path)?; }
    Ok(())
}

pub fn run_monte_carlo(args: MonteCarloArgs) -> Result<()> {
    let config = if let Some(path) = args.input {
        let file = std::fs::File::open(path)?;
        let m: crate::bot::backtest::metrics::BacktestMetrics = serde_json::from_reader(file)?;
        let mut cfg = SimulationConfig::from_metrics(&m);
        cfg.num_simulations = args.simulations;
        cfg.num_trades_per_sim = args.trades;
        cfg.position_size_pct = args.kelly;
        cfg
    } else {
        SimulationConfig {
            num_simulations: args.simulations,
            num_trades_per_sim: args.trades,
            position_size_pct: args.kelly,
            ..Default::default()
        }
    };
    MonteCarloSimulator::new(config).run().print_summary();
    Ok(())
}

pub fn run_parameter_sweep(args: SweepArgs) -> Result<()> {
    let markets = if let Some(path) = args.data {
        let mut p = BeckerParser::new();
        if path.ends_with(".parquet") { p.load_parquet(&path)?; } else { p.load_csv(&path)?; }
        p.organize_by_market();
        p.get_markets().values().cloned().collect()
    } else {
        load_mock_data()
    };
    let bands = vec![(0.25, 0.75), (0.30, 0.70), (0.35, 0.65), (0.40, 0.60), (0.45, 0.55)];
    ParameterSweepResult::print_comparison(&crate::bot::backtest::replay::run_parameter_sweep(&markets, &bands));
    Ok(())
}

pub async fn run_fetch_pmxt(args: FetchPmxtArgs) -> Result<()> {
    let fetcher = PmxtFetcher::new()?.with_filter(&args.filter);
    let rows = fetcher.fetch_url(&args.url)?;
    if rows.is_empty() { return Ok(()); }
    println!("Fetched {} rows", rows.len());
    if let Some(path) = args.csv { PmxtFetcher::export_csv(&rows, &path)?; }
    if let Some(path) = args.parquet { fetcher.export_parquet_direct(&args.url, &path)?; }
    Ok(())
}

pub async fn run_list_markets(args: ListMarketsArgs) -> Result<()> {
    let conn = duckdb::Connection::open_in_memory()?;
    conn.execute_batch("INSTALL httpfs; LOAD httpfs; PRAGMA threads=4;")?;
    let discovered = discover_markets_for_input(&conn, &args.input, args.min_ticks, args.crypto, None, args.verbose).await?;
    if args.table {
        for m in &discovered { println!("{} | {}", m.slug, m.condition_id); }
    } else {
        println!("{}", serde_json::to_string_pretty(&discovered)?);
    }
    Ok(())
}

pub async fn run_extract_midpoints(args: ExtractMidpointsArgs) -> Result<()> {
    let hour = extract_hour_from_filename(&args.input);
    let ids = if let Some(ts) = hour { probe_btc_updown_slugs(ts).await? } else { fetch_btc_condition_ids(&args.filter).await? };
    if ids.is_empty() { return Ok(()); }
    
    let conn = duckdb::Connection::open_in_memory()?;
    conn.execute_batch("INSTALL httpfs; LOAD httpfs;")?;
    let id_list = ids.iter().map(|(id, _)| format!("'{}'", id)).collect::<Vec<_>>().join(",");
    let sql = format!("SELECT market_id, COALESCE(TRY_CAST(data->>'$.timestamp' AS DOUBLE), 0.0) as ts, (COALESCE(TRY_CAST(data->>'$.best_bid' AS DOUBLE), 0.0)+COALESCE(TRY_CAST(data->>'$.best_ask' AS DOUBLE), 0.0))/2.0 as mid, COALESCE(TRY_CAST(data->>'$.best_bid' AS DOUBLE), 0.0) as bid, COALESCE(TRY_CAST(data->>'$.best_ask' AS DOUBLE), 0.0) as ask FROM read_parquet('{}') WHERE market_id IN ({}) ORDER BY ts", args.input, id_list);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| Ok(MidpointRow {
        market_id: row.get(0)?,
        market_slug: String::new(),
        timestamp: row.get::<_, f64>(1)? as i64,
        midpoint: row.get::<_, f64>(2)?,
        best_bid: row.get::<_, f64>(3)?,
        best_ask: row.get::<_, f64>(4)?,
    }))?;
    let results: Vec<MidpointRow> = rows.collect::<Result<Vec<_>, _>>()?;
    if let Some(path) = args.csv {
        let mut w = csv::Writer::from_path(path)?;
        for r in results { w.serialize(r)?; }
    }
    Ok(())
}

pub fn run_inspect_parquet(args: InspectParquetArgs) -> Result<()> {
    let conn = duckdb::Connection::open_in_memory()?;
    conn.execute_batch("INSTALL httpfs; LOAD httpfs;")?;
    let sql = format!("SELECT * FROM read_parquet('{}') LIMIT {}", args.input, args.sample);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| Ok(row.get::<_, Option<String>>(0).unwrap_or_default()))?;
    for (i, r) in rows.enumerate() { println!("{}: {:?}", i, r?); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_top_n_discovered_markets, CryptoAsset, ParquetMarketStat, ReplayMode, ReplayRow,
        build_replay_snapshots,
    };

    #[test]
    fn top_n_discovery_uses_tick_ranking_and_synthetic_btc_slugs() {
        let discovered = build_top_n_discovered_markets(
            vec![
                ParquetMarketStat {
                    condition_id: "c-low".to_string(),
                    ticks: 10,
                    min_ts: 1_700_000_000.0,
                    max_ts: 1_700_000_100.0,
                },
                ParquetMarketStat {
                    condition_id: "c-high".to_string(),
                    ticks: 30,
                    min_ts: 1_700_000_300.0,
                    max_ts: 1_700_000_350.0,
                },
                ParquetMarketStat {
                    condition_id: "c-mid".to_string(),
                    ticks: 20,
                    min_ts: 1_700_000_600.0,
                    max_ts: 1_700_000_620.0,
                },
            ],
            2,
            CryptoAsset::Btc,
            Some("btc-updown-5m"),
        );

        assert_eq!(discovered.len(), 2);
        assert_eq!(discovered[0].condition_id, "c-high");
        assert_eq!(discovered[1].condition_id, "c-mid");
        assert!(discovered.iter().all(|market| market.slug.starts_with("btc-updown-5m-")));
    }

    #[test]
    fn live_parity_replay_emits_one_snapshot_per_second_boundary() {
        let snapshots = build_replay_snapshots(
            &[
                ReplayRow { ts: 1000.1, side: "YES".to_string(), bid: 0.40, ask: 0.42 },
                ReplayRow { ts: 1000.2, side: "NO".to_string(), bid: 0.58, ask: 0.60 },
                ReplayRow { ts: 1001.1, side: "YES".to_string(), bid: 0.43, ask: 0.45 },
                ReplayRow { ts: 1002.2, side: "NO".to_string(), bid: 0.55, ask: 0.57 },
            ],
            ReplayMode::LiveParity1s,
        );

        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0].ts_exchange, 1000.0);
        assert_eq!(snapshots[1].ts_exchange, 1001.0);
        assert_eq!(snapshots[2].ts_exchange, 1002.0);
    }
}
