use anyhow::{Context, Result};
use chrono::{DateTime, Timelike, Utc};
use clap::{Args, ValueEnum};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::request::{
    MidpointRequest, OrderBookSummaryRequest, PriceRequest,
};
use polymarket_client_sdk::clob::types::response::MarketResponse;
use polymarket_client_sdk::gamma;
use polymarket_client_sdk::gamma::types::request::{
    MarketBySlugRequest, MarketsRequest, SearchRequest,
};
use polymarket_client_sdk::gamma::types::response::Market;
use polymarket_client_sdk::types::{Decimal, U256};
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Duration};

use crate::bot::feed::MarketSnapshot;

// ── Constants ──────────────────────────────────────────────────────────────────

pub use crate::bot::pipeline::{
    BTC_UPDOWN_SLUG_PREFIX, FIVE_MINUTES_SECONDS,
    gamma_market_condition_id_hex, hour_btc_5m_slugs, is_updown_5m_text,
    market_matches_exact_filter, matches_crypto_text,
    has_binary_directional_tokens, infer_market_window, parse_slug_timestamp,
    window_overlaps_hour,
};
pub const BTC_UPDOWN_15M_SLUG_PREFIX: &str = "btc-updown-15m-";
pub const FIFTEEN_MINUTES_SECONDS: i64 = 900;

// ── Enums ──────────────────────────────────────────────────────────────────────

pub use crate::bot::pipeline::CryptoAsset;

// ── Structs ────────────────────────────────────────────────────────────────────

pub struct WatchedMarket {
    pub label: String,
    pub slug: String,
    pub yes_token_id: U256,
    pub no_token_id: U256,
    pub condition_id: Option<String>,
    pub end_time: DateTime<Utc>,
}

pub use crate::bot::pipeline::DiscoveredMarket;

pub use crate::bot::pipeline::GammaMarket;

// ── CLI arg structs ────────────────────────────────────────────────────────────

// ── Live discovery ─────────────────────────────────────────────────────────────

pub async fn discover_market_loop(client: &gamma::Client) -> WatchedMarket {
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

pub async fn discover_active_btc_market(client: &gamma::Client) -> Result<WatchedMarket> {
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

pub fn candidate_slug_timestamps(now_ts: i64) -> Vec<i64> {
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

pub fn is_btc_updown_slug_or_question(market: &Market) -> bool {
    market
        .slug
        .as_deref()
        .is_some_and(|slug| slug.starts_with(BTC_UPDOWN_SLUG_PREFIX))
        || market.question.as_deref().is_some_and(is_btc_up_down_5m)
}

pub fn is_btc_up_down_5m(question: &str) -> bool {
    let normalized = question.to_ascii_lowercase();
    (normalized.contains("btc") || normalized.contains("bitcoin"))
        && normalized.contains("up")
        && normalized.contains("down")
        && (normalized.contains("5m")
            || normalized.contains("5 min")
            || normalized.contains("5-minute")
            || normalized.contains("five minute"))
}

pub fn is_active_now(market: &Market, now: &DateTime<Utc>) -> bool {
    if market.closed == Some(true) || market.active == Some(false) {
        return false;
    }

    let starts_ok = market.start_date.as_ref().is_none_or(|start| start <= now);
    let ends_ok = market.end_date.as_ref().is_some_and(|end| end > now);
    starts_ok && ends_ok
}

pub fn market_to_watched(market: Market) -> Result<WatchedMarket> {
    let (yes_token_id, no_token_id) = select_binary_tokens(&market)?;
    let fallback_slug = format!("market-{}", market.id);
    let end_time = market
        .end_date
        .context("market end date is missing")?;

    let condition_id = market.condition_id.map(|c| format!("0x{}", alloy::hex::encode(c.as_slice())));

    Ok(WatchedMarket {
        label: market_label(&market),
        slug: market.slug.unwrap_or(fallback_slug),
        yes_token_id,
        no_token_id,
        condition_id,
        end_time,
    })
}

pub fn select_binary_tokens(market: &Market) -> Result<(U256, U256)> {
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

pub fn market_label(market: &Market) -> String {
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

pub fn btc_updown_label_from_question(question: &str) -> String {
    question.split_once(" - ").map_or_else(
        || question.to_string(),
        |(_, suffix)| format!("BTC 5m {suffix}"),
    )
}

// ── REST snapshot (used by poll-mode live feed) ────────────────────────────────

pub async fn fetch_snapshot(
    client: &clob::Client,
    token_id: U256,
) -> Result<MarketSnapshot> {
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
        .side(polymarket_client_sdk::clob::types::Side::Sell)
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
        .side(polymarket_client_sdk::clob::types::Side::Buy)
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


// Migrated to crate::bot::pipeline


// Migrated to crate::bot::pipeline



