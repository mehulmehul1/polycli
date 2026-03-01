use crate::bot::candles::CandleEngine;
use crate::bot::indicators::{IndicatorEngine, IndicatorState};
use crate::bot::signal::{SignalEngine, EntrySignal, ExitSignal};
use crate::bot::validation::ValidationTracker;
use anyhow::{Context, Result};
use chrono::{DateTime, Timelike, Utc};
use clap::{Args, Subcommand};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::request::{MidpointRequest, OrderBookSummaryRequest, PriceRequest};
use polymarket_client_sdk::clob::types::Side;
use polymarket_client_sdk::gamma;
use polymarket_client_sdk::gamma::types::request::{
    MarketBySlugRequest, MarketsRequest, SearchRequest,
};
use polymarket_client_sdk::gamma::types::response::Market;
use polymarket_client_sdk::types::{Decimal, U256};
use tokio::time::{Duration, MissedTickBehavior, interval, sleep};

const BTC_UPDOWN_SLUG_PREFIX: &str = "btc-updown-5m-";
const FIVE_MINUTES_SECONDS: i64 = 300;

#[derive(Args)]
pub struct BotArgs {
    #[command(subcommand)]
    pub command: BotCommand,
}

#[derive(Subcommand)]
pub enum BotCommand {
    /// Watch the active 5-minute BTC "Up or Down" market and print live orderbook stats
    WatchBtc,
    /// Automated 20-market validation run with metrics export
    ValidateBtc,
}

struct WatchedMarket {
    label: String,
    slug: String,
    yes_token_id: U256,
    no_token_id: U256,
    end_time: DateTime<Utc>,
}

struct MarketSnapshot {
    midpoint: Option<Decimal>,
    best_bid: Option<Decimal>,
    best_ask: Option<Decimal>,
    spread: Option<Decimal>,
    top5_bid_depth: Decimal,
    top5_ask_depth: Decimal,
}

struct DualSnapshot {
    yes: MarketSnapshot,
    no: MarketSnapshot,
}

#[derive(Debug, Clone, Copy)]
enum TokenSide {
    Yes,
    No,
}

struct ShadowPosition {
    active_entry: Option<EntrySignal>,
    token_side: Option<TokenSide>,
    entry_price: f64,
    size: f64,
    realized_pnl: f64,
    position_realized_pnl: f64,
    last_exit_timestamp: u64,
    entry_timestamp: u64,
    position_size_usd: f64,
    bankroll_usd: f64,
    realized_usd: f64,
    position_realized_usd: f64,
    // Directional lock: prevent re-entry after loss in same direction
    yes_blocked: bool,
    no_blocked: bool,
}

impl Default for ShadowPosition {
    fn default() -> Self {
        Self {
            active_entry: None,
            token_side: None,
            entry_price: 0.0,
            size: 0.0,
            realized_pnl: 0.0,
            position_realized_pnl: 0.0,
            last_exit_timestamp: 0,
            entry_timestamp: 0,
            position_size_usd: 0.0,
            bankroll_usd: 4.0,
            realized_usd: 0.0,
            position_realized_usd: 0.0,
            yes_blocked: false,
            no_blocked: false,
        }
    }
}

impl ShadowPosition {
    fn is_active(&self) -> bool {
        self.token_side.is_some()
    }

    fn reset(&mut self, timestamp: u64) {
        // Block this direction if trade was a loss
        if self.position_realized_pnl < 0.0 {
            match self.token_side {
                Some(TokenSide::Yes) => self.yes_blocked = true,
                Some(TokenSide::No) => self.no_blocked = true,
                None => {}
            }
        }
        
        self.active_entry = None;
        self.token_side = None;
        self.entry_price = 0.0;
        self.size = 0.0;
        self.position_realized_pnl = 0.0;
        self.last_exit_timestamp = timestamp;
        self.position_size_usd = 0.0;
        self.position_realized_usd = 0.0;
    }

    fn full_reset(&mut self) {
        self.active_entry = None;
        self.token_side = None;
        self.entry_price = 0.0;
        self.size = 0.0;
        self.position_realized_pnl = 0.0;
        self.last_exit_timestamp = 0;
        self.entry_timestamp = 0;
        self.position_size_usd = 0.0;
        // bankroll_usd carries over - do NOT reset
        self.position_realized_usd = 0.0;
        // Clear directional blocks for new contract
        self.yes_blocked = false;
        self.no_blocked = false;
    }

    fn pnl(&self, current_price: f64) -> f64 {
        if !self.is_active() || self.entry_price < 0.0001 {
            return 0.0;
        }
        (current_price - self.entry_price) / self.entry_price
    }
}

pub async fn execute(args: BotArgs) -> Result<()> {
    match args.command {
        BotCommand::WatchBtc => watch_btc_market(None).await,
        BotCommand::ValidateBtc => watch_btc_market(Some(20)).await,
    }
}

async fn watch_btc_market(max_markets: Option<usize>) -> Result<()> {
    let gamma_client = gamma::Client::default();
    let clob_client = clob::Client::default();

    let mut watched = discover_market_loop(&gamma_client).await;

    let mut validator = max_markets.map(ValidationTracker::new);

    let mut ind_1m = IndicatorEngine::new();
    let mut ind_5s = IndicatorEngine::new();
    let mut signal_engine = SignalEngine::new();

    let mut candle_engine = CandleEngine::new();
    candle_engine.set_debug(false);

    let mut shadow = ShadowPosition::default();

    let mut state_1m = IndicatorState::default();
    let mut state_5s = IndicatorState::default();

    let mut last_midpoint = None;
    let mut last_yes_bid = 0.0;
    let mut last_no_bid = 0.0;
    let mut current_slug = watched.slug.clone();

    let mut ticker = interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    println!("[SHADOW MODE] Probability Expansion Scalper");
    println!("[SHADOW MODE] Entry: slope > 0.002 + breakout | Exit: slope flip");
    println!("[SHADOW MODE] Range: 0.35 - 0.65 | Spread: < 10% of ask");
    println!("========================================");

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
                        // Use correct side's bid for settlement (not YES midpoint for all!)
                        let price = match shadow.token_side {
                            Some(TokenSide::Yes) => last_yes_bid,
                            Some(TokenSide::No) => last_no_bid,
                            None => shadow.entry_price,
                        };
                        
                        // If price is near 0, the side lost - settle at 0
                        // If price is near 1, the side won - settle at 1
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
                            "[SETTLEMENT] {} | {} @ {:.2} → {:.2} | {:.4}% | Bankroll: ${:.2}",
                            side_name, watched.slug, shadow.entry_price, settlement_price, 
                            shadow.position_realized_pnl * 100.0, shadow.bankroll_usd
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

                    signal_engine.reset();
                    ind_1m.reset();
                    ind_5s.reset();
                    shadow.full_reset();
                    state_1m = IndicatorState::default();
                    state_5s = IndicatorState::default();
                    last_midpoint = None;
                    last_yes_bid = 0.0;
                    last_no_bid = 0.0;

                    println!("[MARKET RESET] All engines cleared | {}", watched.slug);
                    println!("========================================");
                }

                let yes_snapshot = match fetch_snapshot(&clob_client, watched.yes_token_id).await {
                    Ok(s) => s,
                    Err(err) => {
                        eprintln!("[warn] Failed to fetch YES market data: {err:#}");
                        continue;
                    }
                };
                let no_snapshot = match fetch_snapshot(&clob_client, watched.no_token_id).await {
                    Ok(s) => s,
                    Err(err) => {
                        eprintln!("[warn] Failed to fetch NO market data: {err:#}");
                        continue;
                    }
                };

                let dual_snapshot = DualSnapshot {
                    yes: yes_snapshot,
                    no: no_snapshot,
                };

                if let Some(midpoint) = midpoint_price(&dual_snapshot.yes) {
                    last_midpoint = Some(midpoint);
                    last_yes_bid = best_bid_price(&dual_snapshot.yes).unwrap_or(last_yes_bid);
                    last_no_bid = best_bid_price(&dual_snapshot.no).unwrap_or(last_no_bid);
                    let simulated_volume = decimal_to_f64(dual_snapshot.yes.top5_bid_depth + dual_snapshot.yes.top5_ask_depth);
                    let spread_f64 = dual_snapshot.yes.spread.map(decimal_to_f64).unwrap_or(0.0);
                    let epoch_seconds = Utc::now().timestamp() as u64;

                    // Periodic book state output every 10s
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

                    let closed_candles = candle_engine.update(midpoint, spread_f64, simulated_volume, epoch_seconds);

                    if let Some(closed) = closed_candles {
                        if let Some(c) = closed.one_minute {
                            state_1m = ind_1m.update(&c);
                        }

                        if let Some(c) = closed.five_second {
                            state_5s = ind_5s.update(&c);

                            let mut signal = signal_engine.update(
                                &state_5s,
                                &state_1m,
                                midpoint
                            );

                            if signal.entry != EntrySignal::None {
                                if let Some(v) = &mut validator {
                                    v.record_signal();
                                }
                            }

                            if signal.entry != EntrySignal::None {
                                let contract_age = (epoch_seconds as i64) - (watched.end_time.timestamp() - FIVE_MINUTES_SECONDS);
                                let time_remaining = (watched.end_time.timestamp() - epoch_seconds as i64).max(0);
                                
                                let snapshot_side = match signal.entry {
                                    EntrySignal::Long => &dual_snapshot.yes,
                                    EntrySignal::Short => &dual_snapshot.no,
                                    _ => &dual_snapshot.yes,
                                };

                                let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
                                let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);

                                let side_bid = best_bid_price(snapshot_side).unwrap_or(0.0);
                                let side_ask = best_ask_price(snapshot_side).unwrap_or(0.0);
                                let side_spread = side_ask - side_bid;
                                let max_spread = side_ask * 0.10;

                                if let Err(reason) = trade_allowed(
                                    snapshot_side,
                                    time_remaining,
                                    contract_age,
                                    yes_ask,
                                    no_ask,
                                ) {
                                    println!("[FILTER BLOCKED ENTRY] {} | {} Side | Reason: {:?} | bid={:.4} ask={:.4} spread={:.4} max={:.4}", 
                                        watched.slug, 
                                        if matches!(signal.entry, EntrySignal::Long) { "YES" } else { "NO" }, 
                                        reason,
                                        side_bid,
                                        side_ask,
                                        side_spread,
                                        max_spread
                                    );
                                    signal.entry = EntrySignal::None;
                                    if let Some(v) = &mut validator {
                                        v.record_entry_blocked();
                                    }
                                }
                            }

                            handle_shadow_signals(
                                &signal, 
                                &dual_snapshot, 
                                &mut shadow, 
                                &watched, 
                                epoch_seconds, 
                                &mut validator,
                                midpoint
                            );
                        }
                    }

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
        }
    }
    Ok(())
}

fn handle_shadow_signals(
    signal: &crate::bot::signal::SignalState,
    dual_snapshot: &DualSnapshot,
    shadow: &mut ShadowPosition,
    market: &WatchedMarket,
    timestamp: u64,
    validator: &mut Option<ValidationTracker>,
    midpoint: f64,
) {
    let time_remaining = (market.end_time.timestamp() - Utc::now().timestamp()).max(0);

    if signal.entry != EntrySignal::None && !shadow.is_active() {
        if time_remaining < 30 || time_remaining > 280 {
            return;
        }

        if timestamp - shadow.last_exit_timestamp < 15 {
            return;
        }

        // Directional lock: don't re-enter same direction after loss
        match signal.entry {
            EntrySignal::Long if shadow.yes_blocked => {
                println!("[BLOCKED] YES direction locked after loss");
                return;
            }
            EntrySignal::Short if shadow.no_blocked => {
                println!("[BLOCKED] NO direction locked after loss");
                return;
            }
            _ => {}
        }

        if shadow.bankroll_usd < 1.0 {
            println!("[BANKROLL BLOCKED] ${:.2}", shadow.bankroll_usd);
            return;
        }

        shadow.token_side = match signal.entry {
            EntrySignal::Long => Some(TokenSide::Yes),
            EntrySignal::Short => Some(TokenSide::No),
            EntrySignal::None => None,
        };

        let entry_price = match shadow.token_side {
            Some(TokenSide::Yes) => best_ask_price(&dual_snapshot.yes),
            Some(TokenSide::No) => best_ask_price(&dual_snapshot.no),
            _ => None,
        };

        match entry_price {
            Some(price) if price > 0.0001 => {
                shadow.active_entry = Some(signal.entry);
                shadow.entry_price = price;
                shadow.size = 1.0;
                shadow.position_realized_pnl = 0.0;
                shadow.entry_timestamp = timestamp;
                shadow.position_size_usd = 1.0;
                shadow.bankroll_usd -= 1.0;
                shadow.position_realized_usd = 0.0;

                if let Some(v) = validator {
                    v.record_entry_taken();
                }

                let side_name = match shadow.token_side {
                    Some(TokenSide::Yes) => "YES",
                    Some(TokenSide::No) => "NO",
                    None => "N/A",
                };
                
                let yes_bid = best_bid_price(&dual_snapshot.yes).unwrap_or(0.0);
                let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
                let no_bid = best_bid_price(&dual_snapshot.no).unwrap_or(0.0);
                let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);
                
                println!(
                    "[BOOK] YES {:.4}/{:.4} | NO {:.4}/{:.4} | sum={:.4}",
                    yes_bid, yes_ask, no_bid, no_ask, yes_ask + no_ask
                );
                println!(
                    "[ENTRY] {} | {} @ {:.4} (mid={:.4}) | Bankroll: ${:.2}",
                    market.label, side_name, price, midpoint, shadow.bankroll_usd
                );
            }
            _ => {
                println!("[NO LIQUIDITY] No best_ask for {:?}", shadow.token_side);
            }
        }
        return;
    }

    let exit_price = match shadow.token_side {
        Some(TokenSide::Yes) => best_bid_price(&dual_snapshot.yes),
        Some(TokenSide::No) => best_bid_price(&dual_snapshot.no),
        _ => None,
    };

    match signal.exit {
        ExitSignal::FullExit => {
            if shadow.is_active() {
                match exit_price {
                    Some(price) if price > 0.0001 => {
                        let pnl = shadow.pnl(price);
                        let realized = pnl * shadow.size;
                        shadow.realized_pnl += realized;
                        shadow.position_realized_pnl += realized;
                        
                        let dollar_pnl = pnl * shadow.position_size_usd;
                        shadow.bankroll_usd += shadow.position_size_usd + dollar_pnl;
                        shadow.realized_usd += dollar_pnl;
                        shadow.position_realized_usd += dollar_pnl;
                        
                        if let Some(v) = validator {
                            let duration = (timestamp - shadow.entry_timestamp) as i64;
                            let side_str = match shadow.token_side {
                                Some(TokenSide::Yes) => "YES".to_string(),
                                Some(TokenSide::No) => "NO".to_string(),
                                None => "N/A".to_string(),
                            };
                            v.record_trade(
                                market.slug.clone(),
                                side_str,
                                shadow.entry_price,
                                price,
                                shadow.position_realized_pnl,
                                duration,
                                shadow.position_realized_usd,
                                shadow.bankroll_usd,
                            );
                        }

                        println!(
                            "[EXIT SLOPE FLIP] {} | {:.4}% | +${:.4} | Bankroll: ${:.2}",
                            market.label, shadow.position_realized_pnl * 100.0, shadow.position_realized_usd, shadow.bankroll_usd
                        );
                        shadow.reset(timestamp);
                    }
                    _ => {
                        println!("[NO EXIT BID] {:?}", shadow.token_side);
                    }
                }
            }
        }
        ExitSignal::None => {}
    }
}

async fn discover_market_loop(client: &gamma::Client) -> WatchedMarket {
    loop {
        match discover_active_btc_market(client).await {
            Ok(market) => {
                println!(
                    "Watching: {} [{}] (YES {}, NO {})",
                    market.label, market.slug, market.yes_token_id, market.no_token_id
                );
                return market;
            }
            Err(err) => {
                eprintln!("[warn] Could not find active BTC 5m market: {err:#}");
                sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn discover_active_btc_market(client: &gamma::Client) -> Result<WatchedMarket> {
    if let Some(market) = discover_by_time_slugs(client).await? {
        return market_to_watched(market);
    }

    let mut candidates = search_candidates(client).await?;
    if candidates.is_empty() {
        candidates = list_open_market_candidates(client).await?;
    }

    let now = Utc::now();
    let market = candidates
        .into_iter()
        .filter(is_btc_updown_slug_or_question)
        .filter(|market| is_active_now(market, &now))
        .min_by_key(|market| market.end_date);

    match market {
        Some(market) => market_to_watched(market),
        None => Err(anyhow::anyhow!(
            "no matching active BTC 5m market found"
        )),
    }
}

async fn discover_by_time_slugs(client: &gamma::Client) -> Result<Option<Market>> {
    let now = Utc::now();
    let mut active: Vec<Market> = Vec::new();

    for ts in candidate_slug_timestamps(now.timestamp()) {
        let slug = format!("{BTC_UPDOWN_SLUG_PREFIX}{ts}");
        let request = MarketBySlugRequest::builder().slug(slug).build();

        match client.market_by_slug(&request).await {
            Ok(market) => {
                if is_active_now(&market, &now) {
                    active.push(market);
                }
            }
            Err(err) => {
                if !err.to_string().contains("404") {
                    eprintln!("[warn] slug probe failed: {err}");
                }
            }
        }
    }

    Ok(active.into_iter().min_by_key(|market| market.end_date))
}

fn candidate_slug_timestamps(now_ts: i64) -> Vec<i64> {
    let base = now_ts.div_euclid(FIVE_MINUTES_SECONDS) * FIVE_MINUTES_SECONDS;
    [-900, -600, -300, 0, 300, 600, 900]
        .into_iter()
        .map(|offset| base + offset)
        .collect()
}

async fn search_candidates(client: &gamma::Client) -> Result<Vec<Market>> {
    let request = SearchRequest::builder()
        .q("btc-updown-5m")
        .limit_per_type(50)
        .build();
    let results = client.search(&request).await?;
    Ok(results
        .events
        .unwrap_or_default()
        .into_iter()
        .flat_map(|event| event.markets.unwrap_or_default())
        .collect())
}

async fn list_open_market_candidates(client: &gamma::Client) -> Result<Vec<Market>> {
    let request = MarketsRequest::builder().limit(200).closed(false).build();
    Ok(client.markets(&request).await?)
}

fn is_btc_updown_slug_or_question(market: &Market) -> bool {
    market
        .slug
        .as_deref()
        .is_some_and(|slug| slug.starts_with(BTC_UPDOWN_SLUG_PREFIX))
        || market.question.as_deref().is_some_and(is_btc_up_down_5m)
}

fn is_btc_up_down_5m(question: &str) -> bool {
    let normalized = question.to_ascii_lowercase();
    (normalized.contains("btc") || normalized.contains("bitcoin"))
        && normalized.contains("up")
        && normalized.contains("down")
        && (normalized.contains("5m")
            || normalized.contains("5 min")
            || normalized.contains("5-minute")
            || normalized.contains("five minute"))
}

fn is_active_now(market: &Market, now: &DateTime<Utc>) -> bool {
    if market.closed == Some(true) || market.active == Some(false) {
        return false;
    }

    let starts_ok = market.start_date.as_ref().is_none_or(|start| start <= now);
    let ends_ok = market.end_date.as_ref().is_some_and(|end| end > now);
    starts_ok && ends_ok
}

fn market_to_watched(market: Market) -> Result<WatchedMarket> {
    let (yes_token_id, no_token_id) = select_binary_tokens(&market)?;
    let fallback_slug = format!("market-{}", market.id);
    let end_time = market
        .end_date
        .context("market end date is missing")?;

    Ok(WatchedMarket {
        label: market_label(&market),
        slug: market.slug.unwrap_or(fallback_slug),
        yes_token_id,
        no_token_id,
        end_time,
    })
}

fn select_binary_tokens(market: &Market) -> Result<(U256, U256)> {
    let outcomes = market
        .outcomes
        .as_ref()
        .context("market outcomes missing")?;

    let token_ids = market
        .clob_token_ids
        .as_ref()
        .context("market CLOB token IDs missing")?;

    if outcomes.len() != token_ids.len() || outcomes.len() != 2 {
        anyhow::bail!("binary market expected exactly 2 outcomes");
    }

    let mut yes_index = None;
    let mut no_index = None;

    for (i, outcome) in outcomes.iter().enumerate() {
        let normalized = outcome.to_ascii_lowercase();
        if normalized.contains("yes") || normalized.contains("up") || normalized.contains("higher") {
            yes_index = Some(i);
        } else {
            no_index = Some(i);
        }
    }

    let yes_index = yes_index.context("YES outcome not found")?;
    let no_index = no_index.context("NO outcome not found")?;

    Ok((token_ids[yes_index], token_ids[no_index]))
}

async fn fetch_snapshot(client: &clob::Client, token_id: U256) -> Result<MarketSnapshot> {
    let midpoint_request = MidpointRequest::builder().token_id(token_id).build();
    let midpoint = match client.midpoint(&midpoint_request).await {
        Ok(resp) => Some(resp.mid),
        Err(err) => {
            eprintln!("[warn] midpoint request failed: {err}");
            None
        }
    };

    // Use price API for actual tradable prices
    // Side::Buy = price to buy at = ask
    // Side::Sell = price to sell at = bid
    let sell_request = PriceRequest::builder()
        .token_id(token_id)
        .side(Side::Sell)
        .build();
    let best_ask = match client.price(&sell_request).await {
        Ok(resp) => Some(resp.price),
        Err(err) => {
            eprintln!("[warn] sell price request failed: {err}");
            None
        }
    };

    let buy_request = PriceRequest::builder()
        .token_id(token_id)
        .side(Side::Buy)
        .build();
    let best_bid = match client.price(&buy_request).await {
        Ok(resp) => Some(resp.price),
        Err(err) => {
            eprintln!("[warn] buy price request failed: {err}");
            None
        }
    };

    let spread = match (best_bid, best_ask) {
        (Some(bid), Some(ask)) => Some(ask - bid),
        _ => None,
    };

    // Get depth from order book (still useful for liquidity assessment)
    let book_request = OrderBookSummaryRequest::builder()
        .token_id(token_id)
        .build();
    let book = match client.order_book(&book_request).await {
        Ok(b) => b,
        Err(err) => {
            eprintln!("[warn] order book request failed: {err}");
            return Ok(MarketSnapshot {
                midpoint,
                best_bid,
                best_ask,
                spread,
                top5_bid_depth: Decimal::ZERO,
                top5_ask_depth: Decimal::ZERO,
            });
        }
    };

    let top5_bid_depth = book
        .bids
        .iter()
        .take(5)
        .fold(Decimal::ZERO, |acc, level| acc + level.size);
    let top5_ask_depth = book
        .asks
        .iter()
        .take(5)
        .fold(Decimal::ZERO, |acc, level| acc + level.size);

    Ok(MarketSnapshot {
        midpoint,
        best_bid,
        best_ask,
        spread,
        top5_bid_depth,
        top5_ask_depth,
    })
}

fn market_label(market: &Market) -> String {
    if market
        .slug
        .as_deref()
        .is_some_and(|slug| slug.starts_with(BTC_UPDOWN_SLUG_PREFIX))
    {
        if let Some(question) = market.question.as_deref() {
            return btc_updown_label_from_question(question);
        }
    }

    if let (Some(start), Some(end)) = (market.start_date.as_ref(), market.end_date.as_ref()) {
        return format!(
            "BTC 5m {:02}:{:02}-{:02}:{:02}",
            start.hour(),
            start.minute(),
            end.hour(),
            end.minute()
        );
    }

    market
        .question
        .clone()
        .unwrap_or_else(|| format!("Market {}", market.id))
}

fn btc_updown_label_from_question(question: &str) -> String {
    question.split_once(" - ").map_or_else(
        || question.to_string(),
        |(_, suffix)| format!("BTC 5m {suffix}"),
    )
}

fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_string().parse::<f64>().unwrap_or_default()
}

#[derive(Debug, PartialEq)]
enum FilterReason {
    NoLiquidity,
    WideSpread,
    ExtremePrice,
    BrokenBook,
    Time,
}

fn trade_allowed(
    snapshot: &MarketSnapshot,
    time_remaining: i64,
    contract_age: i64,
    yes_ask: f64,
    no_ask: f64,
) -> Result<(), FilterReason> {
    let best_bid = snapshot.best_bid.map(decimal_to_f64);
    let best_ask = snapshot.best_ask.map(decimal_to_f64);

    if best_bid.is_none() || best_ask.is_none() {
        return Err(FilterReason::NoLiquidity);
    }

    let bid = best_bid.unwrap();
    let ask = best_ask.unwrap();

    // Dynamic spread cap: 10% of ask price, minimum 3 cents
    // At 0.50: max 0.05, at 0.30: max 0.03, at 0.10: max 0.03 (floor)
    let spread = ask - bid;
    let max_spread = (ask * 0.10).max(0.03);
    if spread > max_spread {
        return Err(FilterReason::WideSpread);
    }

    // Only trade mid-range probabilities (0.35-0.65)
    if ask > 0.65 || ask < 0.35 {
        return Err(FilterReason::ExtremePrice);
    }

    // Complement sanity check: YES + NO should ≈ 1
    if (yes_ask + no_ask - 1.0).abs() > 0.10 {
        return Err(FilterReason::BrokenBook);
    }

    // Need enough time for expansion
    if time_remaining < 30 || contract_age < 15 {
        return Err(FilterReason::Time);
    }

    Ok(())
}

fn best_ask_price(snapshot: &MarketSnapshot) -> Option<f64> {
    snapshot.best_ask.map(decimal_to_f64)
}

fn best_bid_price(snapshot: &MarketSnapshot) -> Option<f64> {
    snapshot.best_bid.map(decimal_to_f64)
}

fn midpoint_price(snapshot: &MarketSnapshot) -> Option<f64> {
    if let Some(mid) = snapshot.midpoint {
        return Some(decimal_to_f64(mid));
    }

    match (snapshot.best_bid, snapshot.best_ask) {
        (Some(bid), Some(ask)) => Some((decimal_to_f64(bid) + decimal_to_f64(ask)) / 2.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_timestamps_are_5m_aligned() {
        let stamps = candidate_slug_timestamps(1_772_188_821);
        assert_eq!(stamps.len(), 7);
        assert!(stamps.into_iter().all(|ts| ts % FIVE_MINUTES_SECONDS == 0));
    }

    #[test]
    fn detects_known_btc_updown_slug_prefix() {
        assert!("btc-updown-5m-1772187900".starts_with(BTC_UPDOWN_SLUG_PREFIX));
    }

    #[test]
    fn question_matcher_handles_variants() {
        assert!(is_btc_up_down_5m("BTC Up or Down in 5m?"));
        assert!(is_btc_up_down_5m(
            "Bitcoin up/down over next 5-minute candle"
        ));
        assert!(!is_btc_up_down_5m("ETH Up or Down in 5m?"));
    }

    #[test]
    fn btc_updown_label_uses_et_question_window() {
        let label =
            btc_updown_label_from_question("Bitcoin Up or Down - February 27, 5:45AM-5:50AM ET");
        assert_eq!(label, "BTC 5m February 27, 5:45AM-5:50AM ET");
    }

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
