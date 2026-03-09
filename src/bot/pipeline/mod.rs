use crate::bot::candles::CandleEngine;
use crate::bot::indicators::{IndicatorEngine, IndicatorState};
use crate::bot::signal::SignalEngine;
use crate::bot::shadow::{ShadowPosition, TokenSide};
use crate::bot::strategy_runner::run_shadow_strategy_step;
use crate::bot::feed::{DualSnapshot, MarketSnapshot, ReplayMode};
use anyhow::{Result, Context};
use chrono::{DateTime, Utc, Timelike};
use serde::{Deserialize, Serialize};
use polymarket_client_sdk::types::{Decimal, U256};
use clap::{Args, ValueEnum};
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
    #[arg(long, value_enum, default_value_t = ReplayMode::Event)]
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
    epoch_seconds: u64,
    yes_bid: f64,
    yes_ask: f64,
    no_bid: f64,
    no_ask: f64,
    candle_engine: &mut CandleEngine,
    ind_1m: &mut IndicatorEngine,
    ind_5s: &mut IndicatorEngine,
    signal_engine: &mut SignalEngine,
    state_1m: &mut IndicatorState,
    state_5s: &mut IndicatorState,
    shadow: &mut ShadowPosition,
    metrics: &mut PipelineMetrics,
    position_size_usd: f64,
    verbose: bool,
) {
    metrics.total_ticks += 1;

    let dual_snapshot = DualSnapshot {
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
        ts_exchange: epoch_seconds as f64,
    };

    if let Some(step) = run_shadow_strategy_step(
        &dual_snapshot,
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
    ) {
        if step.entry_blocked && verbose {
            println!("[PIPELINE] blocked entry {}", market.condition_id);
        }
        if let Some(exit_trade) = step.exit_trade {
            metrics.trades_taken += 1;
            if exit_trade.pnl_usd >= 0.0 {
                metrics.wins += 1;
            } else {
                metrics.losses += 1;
            }
            update_pipeline_capital_metrics(metrics, shadow.bankroll_usd);
        }
    }
}

pub async fn run_backtest_pipeline(args: BacktestPipelineArgs) -> Result<()> {
    let start: DateTime<Utc> = args.start.parse().context("Invalid start time format")?;
    let end: DateTime<Utc> = args.end.parse().context("Invalid end time format")?;

    println!("[PIPELINE] Backtest Pipeline: {} to {}", start, end);
    let inputs = resolve_pipeline_inputs(args.input.as_deref(), start, end);
    
    let conn = duckdb::Connection::open_in_memory()?;
    conn.execute_batch("INSTALL httpfs; LOAD httpfs; PRAGMA threads=4;")?;

    let mut metrics = PipelineMetrics::new(args.capital);
    let mut shadow = ShadowPosition::default();
    shadow.bankroll_usd = args.capital;
    let mut processed_markets = std::collections::HashSet::new();

    let bar = indicatif::ProgressBar::new(inputs.len() as u64);
    for input in &inputs {
        let discovered = discover_markets_for_input(&conn, input, args.min_ticks, args.crypto, Some(&args.filter), args.verbose).await?;
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
            shadow.full_reset();

            let tick_sql = format!(
                "SELECT CAST(data->>'$.timestamp' AS DOUBLE) as ts, UPPER(data->>'$.side') as side, CAST(data->>'$.best_bid' AS DOUBLE) as bid, CAST(data->>'$.best_ask' AS DOUBLE) as ask FROM read_parquet('{}') WHERE market_id = '{}' ORDER BY ts",
                input, market.condition_id
            );
            let mut stmt = conn.prepare(&tick_sql)?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, f64>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?, row.get::<_, f64>(3)?)))?;

            let (mut yes_bid, mut yes_ask, mut no_bid, mut no_ask) = (0.0, 0.0, 0.0, 0.0);
            let mut pending_second: Option<u64> = None;

            for row in rows {
                let (ts, side, bid, ask) = row?;
                match side.as_str() {
                    "YES" => { yes_bid = bid; yes_ask = ask; }
                    "NO" => { no_bid = bid; no_ask = ask; }
                    _ => continue,
                }
                if yes_bid <= 0.0 || no_bid <= 0.0 { continue; }

                let row_second = ts as u64;
                if let Some(prev) = pending_second {
                    if row_second > prev {
                        for s in prev..row_second {
                            process_pipeline_snapshot(&market, s, yes_bid, yes_ask, no_bid, no_ask, &mut candle_engine, &mut ind_1m, &mut ind_5s, &mut signal_engine, &mut state_1m, &mut state_5s, &mut shadow, &mut metrics, args.size, args.verbose);
                        }
                    }
                }
                pending_second = Some(row_second);
            }
            if shadow.is_active() {
                let side_bid = if shadow.token_side == Some(TokenSide::Yes) { yes_bid } else { no_bid };
                let pnl = shadow.pnl(if side_bid > 0.5 { 1.0 } else { 0.0 }) * shadow.position_size_usd;
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
    metrics.print_summary();
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
    let sql = format!("SELECT market_id, CAST(data->>'$.timestamp' AS DOUBLE) as ts, (CAST(data->>'$.best_bid' AS DOUBLE)+CAST(data->>'$.best_ask' AS DOUBLE))/2.0 as mid, CAST(data->>'$.best_bid' AS DOUBLE) as bid, CAST(data->>'$.best_ask' AS DOUBLE) as ask FROM read_parquet('{}') WHERE market_id IN ({}) ORDER BY ts", args.input, id_list);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| Ok(MidpointRow { market_id: row.get(0)?, market_slug: String::new(), timestamp: row.get::<_, f64>(1)? as i64, midpoint: row.get(2)?, best_bid: row.get(3)?, best_ask: row.get(4)? }))?;
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
