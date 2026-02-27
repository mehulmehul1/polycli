use anyhow::{Context, Result};
use chrono::{DateTime, Local, Timelike, Utc};
use clap::{Args, Subcommand};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::request::{MidpointRequest, OrderBookSummaryRequest};
use polymarket_client_sdk::gamma;
use polymarket_client_sdk::gamma::types::request::{
    MarketBySlugRequest, MarketsRequest, SearchRequest,
};
use polymarket_client_sdk::gamma::types::response::Market;
use polymarket_client_sdk::types::{Decimal, U256};
use tokio::time::{interval, sleep, Duration, MissedTickBehavior};

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
}

struct WatchedMarket {
    label: String,
    slug: String,
    yes_token_id: U256,
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

pub async fn execute(args: BotArgs) -> Result<()> {
    match args.command {
        BotCommand::WatchBtc => watch_btc_market().await,
    }
}

async fn watch_btc_market() -> Result<()> {
    let gamma_client = gamma::Client::default();
    let clob_client = clob::Client::default();

    let mut watched = discover_market_loop(&gamma_client).await;
    let mut ticker = interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Received Ctrl+C, stopping bot watch.");
                break;
            }
            _ = ticker.tick() => {
                if Utc::now() >= watched.end_time {
                    println!(
                        "Market {} reached resolution time. Looking for next active BTC 5m market...",
                        watched.slug
                    );
                    watched = discover_market_loop(&gamma_client).await;
                }

                let snapshot = match fetch_snapshot(&clob_client, watched.yes_token_id).await {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        eprintln!("[warn] Failed to fetch market data for {}: {err:#}", watched.slug);
                        continue;
                    }
                };

                print_snapshot(&watched, &snapshot);
            }
        }
    }

    Ok(())
}

async fn discover_market_loop(client: &gamma::Client) -> WatchedMarket {
    loop {
        match discover_active_btc_market(client).await {
            Ok(market) => {
                println!(
                    "Watching market: {} [{}] (YES token {})",
                    market.label, market.slug, market.yes_token_id
                );
                return market;
            }
            Err(err) => {
                eprintln!("[warn] Could not find active BTC 5m market yet: {err:#}");
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
            "no matching active BTC 5m market found by slug probes or fallback search"
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
    let yes_token_id = select_yes_token(&market)?;
    let fallback_slug = format!("market-{}", market.id);
    let end_time = market
        .end_date
        .context("market end date is missing; cannot compute time remaining")?;

    Ok(WatchedMarket {
        label: market_label(&market),
        slug: market.slug.unwrap_or(fallback_slug),
        yes_token_id,
        end_time,
    })
}

fn select_yes_token(market: &Market) -> Result<U256> {
    let outcomes = market
        .outcomes
        .as_ref()
        .context("market outcomes missing")?;
    let token_ids = market
        .clob_token_ids
        .as_ref()
        .context("market CLOB token IDs missing")?;

    if outcomes.len() != token_ids.len() {
        anyhow::bail!(
            "outcomes/token id length mismatch ({} outcomes vs {} token IDs)",
            outcomes.len(),
            token_ids.len()
        );
    }

    let yes_index = outcomes
        .iter()
        .position(|outcome| {
            outcome.eq_ignore_ascii_case("yes")
                || outcome.eq_ignore_ascii_case("up")
                || outcome.eq_ignore_ascii_case("higher")
        })
        .or_else(|| {
            // Some up/down markets may use directional labels; prefer index 0 in a binary market
            // to keep the watcher running instead of failing discovery loops.
            (outcomes.len() == 2).then_some(0)
        })
        .with_context(|| format!("preferred outcome token not found in market outcomes: {outcomes:?}"))?;

    token_ids
        .get(yes_index)
        .copied()
        .context("YES token ID missing at matched outcome index")
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

    let book_request = OrderBookSummaryRequest::builder().token_id(token_id).build();
    let book = client
        .order_book(&book_request)
        .await
        .context("order book request failed")?;

    let best_bid = book.bids.first().map(|order| order.price);
    let best_ask = book.asks.first().map(|order| order.price);
    let spread = match (best_bid, best_ask) {
        (Some(bid), Some(ask)) => Some(ask - bid),
        _ => None,
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

fn print_snapshot(market: &WatchedMarket, snapshot: &MarketSnapshot) {
    let now_local = Local::now();
    let remaining_seconds = (market.end_time - Utc::now()).num_seconds().max(0);

    println!("Time: {}", now_local.format("%H:%M:%S"));
    println!("Market: {}", market.label);
    println!("Mid: {}", display_decimal(snapshot.midpoint));
    println!("Best Bid: {}", display_decimal(snapshot.best_bid));
    println!("Best Ask: {}", display_decimal(snapshot.best_ask));
    println!("Spread: {}", display_decimal(snapshot.spread));
    println!("Bid Depth (top 5): {:.4}", snapshot.top5_bid_depth);
    println!("Ask Depth (top 5): {:.4}", snapshot.top5_ask_depth);
    println!("Time Remaining: {remaining_seconds}s");
    println!("----------------------------------");
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
    question
        .split_once(" - ")
        .map_or_else(|| question.to_string(), |(_, suffix)| format!("BTC 5m {suffix}"))
}

fn display_decimal(value: Option<Decimal>) -> String {
    value
        .map(|v| format!("{v:.4}"))
        .unwrap_or_else(|| "N/A".to_string())
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
        assert!(is_btc_up_down_5m("Bitcoin up/down over next 5-minute candle"));
        assert!(!is_btc_up_down_5m("ETH Up or Down in 5m?"));
    }

    #[test]
    fn btc_updown_label_uses_et_question_window() {
        let label = btc_updown_label_from_question(
            "Bitcoin Up or Down - February 27, 5:45AM-5:50AM ET",
        );
        assert_eq!(label, "BTC 5m February 27, 5:45AM-5:50AM ET");
    }
}



