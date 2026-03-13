//! Execution Module
//!
//! Order execution, fill handling, and feed health monitoring.

use crate::bot::discovery::{WatchedMarket, FIVE_MINUTES_SECONDS, fetch_snapshot};
use crate::bot::feed::DualSnapshot;
use crate::bot::logging::{EngineEvent, EngineEventLoggers};
use crate::bot::risk::{
    best_ask_price, best_bid_price, decimal_to_f64, EntryContext, FilterReason, GateDecision,
    GatekeeperState, TradeDirection,
};
use crate::bot::shadow::TokenSide;
use crate::bot::signal::{EntrySignal, ExitSignal};
use crate::bot::strategy::Direction;
use anyhow::Result;
use chrono::{DateTime, Utc};
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::Normal;
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
use polymarket_client_sdk::clob::types::{Side, OrderType, AssetType, Amount};
use polymarket_client_sdk::types::{Decimal, U256};
use serde::{Deserialize, Serialize};

// ============================================================================
// Execution Types (from execution_spec.md)
// ============================================================================

/// Order type for execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderTypeExt {
    /// Fill-or-Kill: must fill completely or cancel
    FOK,
    /// Fill-and-Kill (Immediate-or-Cancel): partial fills allowed, remainder cancelled
    FAK,
    /// Good-Til-Cancelled: stays on book until filled or cancelled
    GTC,
    /// Good-Til-Date: stays until time expires
    GTD { expiry_ts: u64 },
}

/// Order side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Order request from strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub market_slug: String,
    pub condition_id: String,
    pub direction: Direction,
    pub side: OrderSide,
    pub price: f64,
    pub size_usd: f64,
    pub order_type: OrderTypeExt,
    pub ts: u64,
}

/// Fill result from exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillResult {
    pub order_id: String,
    pub market_slug: String,
    pub direction: Direction,
    pub side: OrderSide,
    pub filled_price: f64,
    pub filled_size_usd: f64,
    pub remaining_size_usd: f64,
    pub fee_usd: f64,
    pub ts: u64,
    pub is_complete: bool,
}

/// Position after fill confirmation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmedPosition {
    pub condition_id: String,
    pub direction: Direction,
    pub entry_ts: u64,
    pub entry_price: f64,
    pub size_usd: f64,
    pub entry_order_id: String,
    pub current_value_usd: f64,
    pub unrealized_pnl_usd: f64,
}

/// Execution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Paper trading (no real orders)
    Shadow,
    /// Dry run (submit to API but don't execute)
    DryRun,
    /// Live execution
    Live,
}

/// Feed health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedHealth {
    Healthy,
    Stale { age_s: u64 },
    Disconnected,
}

impl FeedHealth {
    pub fn is_healthy(&self, max_stale_s: u64) -> bool {
        match self {
            FeedHealth::Healthy => true,
            FeedHealth::Stale { age_s } => *age_s <= max_stale_s,
            FeedHealth::Disconnected => false,
        }
    }
    
    pub fn is_tradeable(&self, market_duration_s: i64) -> bool {
        match self {
            FeedHealth::Healthy => true,
            FeedHealth::Stale { age_s } => {
                // For short markets, any staleness is unacceptable
                market_duration_s > 300 || *age_s < 2
            }
            FeedHealth::Disconnected => false,
        }
    }
}

/// Fill intent for order routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillIntent {
    /// Immediate fill (FAK/IOC)
    Immediate,
    /// Patient fill (GTC with patience)
    Patient,
    /// Must fill completely (FOK)
    Complete,
}

// ============================================================================
// Existing Types
// ============================================================================

pub struct PendingSettlement {
    pub market_slug: String,
    pub token_side: TokenSide,
    pub token_id: U256,
    pub shares: f64,
    pub entry_price: f64,
    pub condition_id: Option<String>,
    pub end_time: DateTime<Utc>,
    pub sell_attempts: u32,
    pub created_at: DateTime<Utc>,
}

pub struct LivePosition {
    pub token_side: Option<TokenSide>,
    pub entry_price: f64,
    pub shares: f64,
    pub entry_timestamp: u64,
    pub last_exit_timestamp: u64,
    pub yes_blocked: bool,
    pub no_blocked: bool,
    pub last_trade_id: Option<String>,
}

impl Default for LivePosition {
    fn default() -> Self {
        Self {
            token_side: None,
            entry_price: 0.0,
            shares: 0.0,
            entry_timestamp: 0,
            last_exit_timestamp: 0,
            yes_blocked: false,
            no_blocked: false,
            last_trade_id: None,
        }
    }
}

impl LivePosition {
    pub fn is_active(&self) -> bool {
        self.token_side.is_some()
    }

    pub fn reset(&mut self, timestamp: u64, was_loss: bool) {
        if was_loss {
            match self.token_side {
                Some(TokenSide::Yes) => self.yes_blocked = true,
                Some(TokenSide::No) => self.no_blocked = true,
                None => {}
            }
        }
        self.token_side = None;
        self.entry_price = 0.0;
        self.shares = 0.0;
        self.entry_timestamp = 0;
        self.last_trade_id = None;
        self.last_exit_timestamp = timestamp;
    }

    pub fn full_reset(&mut self) {
        self.token_side = None;
        self.entry_price = 0.0;
        self.shares = 0.0;
        self.entry_timestamp = 0;
        self.last_exit_timestamp = 0;
        self.last_trade_id = None;
        self.yes_blocked = false;
        self.no_blocked = false;
    }
}

pub struct OrderResult {
    pub order_id: String,
    pub filled_amount: Option<Decimal>,
}

pub async fn get_usdc_balance(client: &clob::Client<Authenticated<Normal>>) -> Result<f64> {
    const USDC_DECIMALS: u32 = 6;
    let request = BalanceAllowanceRequest::builder()
        .asset_type(AssetType::Collateral)
        .build();
    let balance = client.balance_allowance(request).await?;
    let divisor = Decimal::from(10u64.pow(USDC_DECIMALS));
    let human_balance = balance.balance / divisor;
    Ok(decimal_to_f64(human_balance))
}

pub async fn try_settle_pending(
    pending: &mut Vec<PendingSettlement>,
    read_client: &clob::Client,
    clob_client: &clob::Client<Authenticated<Normal>>,
    signer: &(impl polymarket_client_sdk::auth::Signer + Sync),
    gatekeeper: &mut GatekeeperState,
    event_loggers: Option<&EngineEventLoggers>,
    now: DateTime<Utc>,
) {
    let mut settled = Vec::new();

    for i in 0..pending.len() {
        let p = &mut pending[i];
        
        // Only try to sell every 5 seconds
        if p.sell_attempts > 0 && (now - p.created_at).num_seconds() % 5 != 0 {
            continue;
        }

        p.sell_attempts += 1;

        // Fetch current bid for the token
        let snapshot = match fetch_snapshot(read_client, p.token_id).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        let bid_price = match snapshot.best_bid.map(decimal_to_f64) {
            Some(price) if price > 0.0 => price,
            _ => {
                // No liquidity yet
                if p.sell_attempts % 10 == 0 {
                    println!("[PENDING] {} | {:?} | No bid after {} attempts", 
                        p.market_slug, p.token_side, p.sell_attempts);
                }
                continue;
            }
        };

        // If bid is >= 0.95, try to sell (winning token post-resolution)
        // Or if bid is reasonable and we've waited > 60s
        let elapsed = (now - p.created_at).num_seconds();
        let should_sell = bid_price >= 0.95 || (bid_price >= 0.90 && elapsed > 60) || (bid_price > 0.0 && elapsed > 120);

        if !should_sell {
            if p.sell_attempts % 10 == 0 {
                println!("[PENDING] {} | {:?} | Bid {:.4} too low, waiting...", 
                    p.market_slug, p.token_side, bid_price);
            }
            continue;
        }

        // Query ACTUAL token balance (don't trust tracked shares)
        let balance_request = BalanceAllowanceRequest::builder()
            .asset_type(AssetType::Conditional)
            .token_id(p.token_id)
            .build();
        
        let actual_balance = match clob_client.balance_allowance(balance_request).await {
            Ok(b) => {
                // Balance is in raw units (micro-shares), divide by 10^6
                const DECIMALS: u32 = 6;
                let divisor = Decimal::from(10u64.pow(DECIMALS));
                let human_balance = b.balance / divisor;
                decimal_to_f64(human_balance)
            }
            Err(err) => {
                eprintln!("[AUTO-SELL] {} | Failed to fetch balance: {:?}", p.market_slug, err);
                continue;
            }
        };

        // Round DOWN to 2 decimals
        let shares_to_sell = (actual_balance * 100.0).floor() / 100.0;
        
        if shares_to_sell < 0.01 {
            println!("[SETTLED] {} | {:?} | No shares left (balance: {:.6})", 
                p.market_slug, p.token_side, actual_balance);
            settled.push(i);
            continue;
        }

        let side_name = match p.token_side {
            TokenSide::Yes => "YES",
            TokenSide::No => "NO",
        };

        println!(
            "[AUTO-SELL] {} | {:?} | {:.4} shares @ {:.4} (actual: {:.6})",
            p.market_slug, p.token_side, shares_to_sell, bid_price, actual_balance
        );

        match place_market_sell(clob_client, signer, p.token_id, shares_to_sell).await {
            Ok(result) => {
                let pnl_pct = (bid_price - p.entry_price) / p.entry_price * 100.0;
                let pnl_usd = pnl_pct / 100.0 * (shares_to_sell * p.entry_price);
                gatekeeper.record_trade_result(now.timestamp() as u64, pnl_usd);
                println!(
                    "[SETTLED] {} | {} | {:.2}% | ${:.2} | Order: {}",
                    p.market_slug, side_name, pnl_pct, pnl_usd, result.order_id
                );
                if let Some(loggers) = event_loggers {
                    loggers.log_execution(EngineEvent::PendingSettlement {
                        ts: now.timestamp() as u64,
                        market_slug: p.market_slug.clone(),
                        side: side_name.to_string(),
                        bid_price,
                        shares: shares_to_sell,
                    });
                    loggers.log_execution(EngineEvent::LiveExit {
                        ts: now.timestamp() as u64,
                        market_slug: p.market_slug.clone(),
                        side: side_name.to_string(),
                        price: bid_price,
                        pnl_usd,
                        order_id: Some(result.order_id.clone()),
                    });
                    if gatekeeper.emergency_halt {
                        loggers.log_execution(EngineEvent::EmergencyHalt {
                            ts: now.timestamp() as u64,
                            market_slug: p.market_slug.clone(),
                            daily_pnl: gatekeeper.daily_pnl,
                            reason: "daily loss limit".to_string(),
                        });
                    }
                }
                settled.push(i);
            }
            Err(err) => {
                eprintln!("[AUTO-SELL FAILED] {} | {:?} | {:?}", p.market_slug, p.token_side, err);
            }
        }
    }

    // Remove settled positions (in reverse order to maintain indices)
    for i in settled.into_iter().rev() {
        pending.remove(i);
    }
}

pub async fn place_market_buy(
    client: &clob::Client<Authenticated<Normal>>,
    signer: &(impl polymarket_client_sdk::auth::Signer + Sync),
    token_id: U256,
    amount_usd: f64,
) -> Result<OrderResult> {
    let amount = Amount::usdc(Decimal::try_from(amount_usd)?)?;
    
    let order = client
        .market_order()
        .token_id(token_id)
        .side(Side::Buy)
        .amount(amount)
        .order_type(OrderType::FOK)
        .build()
        .await?;

    let signed_order = client.sign(signer, order).await?;
    let result = client.post_order(signed_order).await?;

    Ok(OrderResult {
        order_id: result.order_id,
        filled_amount: Some(result.taking_amount),
    })
}

pub async fn place_market_sell(
    client: &clob::Client<Authenticated<Normal>>,
    signer: &(impl polymarket_client_sdk::auth::Signer + Sync),
    token_id: U256,
    shares: f64,
) -> Result<OrderResult> {
    // Round DOWN to 2 decimal places (API limit) - never try to sell more than we have
    let shares_rounded = (shares * 100.0).floor() / 100.0;
    if shares_rounded < 0.01 {
        anyhow::bail!("Shares too small to sell: {}", shares);
    }
    let amount = Amount::shares(Decimal::try_from(shares_rounded)?)?;
    
    let order = client
        .market_order()
        .token_id(token_id)
        .side(Side::Sell)
        .amount(amount)
        .order_type(OrderType::FOK)
        .build()
        .await?;

    let signed_order = client.sign(signer, order).await?;
    let result = client.post_order(signed_order).await?;

    Ok(OrderResult {
        order_id: result.order_id,
        filled_amount: Some(result.taking_amount),
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_live_signals(
    signal: &crate::bot::signal::SignalState,
    dual_snapshot: &DualSnapshot,
    position: &mut LivePosition,
    gatekeeper: &mut GatekeeperState,
    event_loggers: Option<&EngineEventLoggers>,
    market: &WatchedMarket,
    timestamp: u64,
    size_usd: f64,
    dry_run: bool,
    clob_client: &clob::Client<Authenticated<Normal>>,
    signer: &(impl polymarket_client_sdk::auth::Signer + Sync),
) {
    let time_remaining = (market.end_time.timestamp() - Utc::now().timestamp()).max(0);

    if signal.entry != EntrySignal::None && !position.is_active() {
        let (token_id, token_side, snapshot_side) = match signal.entry {
            EntrySignal::Long => (market.yes_token_id, TokenSide::Yes, &dual_snapshot.yes),
            EntrySignal::Short => (market.no_token_id, TokenSide::No, &dual_snapshot.no),
            EntrySignal::None => return,
        };
        let direction = match signal.entry {
            EntrySignal::Long => TradeDirection::Yes,
            EntrySignal::Short => TradeDirection::No,
            EntrySignal::None => return,
        };
        let direction_locked = match signal.entry {
            EntrySignal::Long => position.yes_blocked,
            EntrySignal::Short => position.no_blocked,
            EntrySignal::None => false,
        };

        let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
        let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);

        let contract_age = (timestamp as i64) - (market.end_time.timestamp() - FIVE_MINUTES_SECONDS);
        let decision = gatekeeper.check_entry(
            snapshot_side,
            &EntryContext {
                timestamp,
                time_remaining,
                min_time_remaining: 30,
                max_time_remaining: 280,
                contract_age,
                yes_ask,
                no_ask,
                bankroll_available: size_usd,
                position_size_usd: size_usd,
                direction_locked,
                direction,
            },
        );

        if let Some(loggers) = event_loggers {
            match &decision {
                GateDecision::Approved { reason } => loggers.log_strategy(EngineEvent::GateDecision {
                    ts: timestamp,
                    market_slug: market.slug.clone(),
                    decision: "approved".to_string(),
                    reason: reason.clone(),
                }),
                GateDecision::Blocked { reason } => {
                    loggers.log_strategy(EngineEvent::GateDecision {
                        ts: timestamp,
                        market_slug: market.slug.clone(),
                        decision: "blocked".to_string(),
                        reason: format!("{reason:?}"),
                    });
                    if matches!(reason, FilterReason::EmergencyHalt | FilterReason::DailyLossLimit) {
                        loggers.log_execution(EngineEvent::EmergencyHalt {
                            ts: timestamp,
                            market_slug: market.slug.clone(),
                            daily_pnl: gatekeeper.daily_pnl,
                            reason: format!("{reason:?}"),
                        });
                    }
                }
            }
        }

        if let GateDecision::Blocked { reason } = decision {
            println!("[FILTER BLOCKED] {:?} | Reason: {:?}", token_side, reason);
            return;
        }

        let entry_price = best_ask_price(snapshot_side).unwrap_or(0.0);
        if entry_price < 0.0001 {
            println!("[NO LIQUIDITY] No ask price for {:?}", token_side);
            return;
        }

        let side_name = match token_side {
            TokenSide::Yes => "YES",
            TokenSide::No => "NO",
        };

        println!(
            "[SIGNAL] {} | BUY {} @ {:.4}",
            market.label, side_name, entry_price
        );

        if dry_run {
            println!("[DRY RUN] Would place market buy order: ${:.2} USDC for {}", size_usd, side_name);
            position.token_side = Some(token_side);
            position.entry_price = entry_price;
            position.shares = (size_usd / entry_price * 100.0).floor() / 100.0;
            position.entry_timestamp = timestamp;
            if let Some(loggers) = event_loggers {
                loggers.log_execution(EngineEvent::LiveEntry {
                    ts: timestamp,
                    market_slug: market.slug.clone(),
                    side: side_name.to_string(),
                    price: entry_price,
                    size_usd,
                    order_id: None,
                });
            }
            return;
        }

        match place_market_buy(clob_client, signer, token_id, size_usd).await {
            Ok(order_result) => {
                let filled_size = order_result.filled_amount.map(decimal_to_f64).unwrap_or(size_usd / entry_price);
                // Round DOWN to avoid trying to sell more than we have
                let filled_rounded = (filled_size * 100.0).floor() / 100.0;
                position.token_side = Some(token_side);
                position.entry_price = entry_price;
                position.shares = filled_rounded;
                position.entry_timestamp = timestamp;
                position.last_trade_id = Some(order_result.order_id.clone());

                println!(
                    "[ORDER FILLED] {} | {} @ {:.4} | {} shares | Order: {}",
                    market.label, side_name, entry_price, filled_rounded, order_result.order_id
                );
                if let Some(loggers) = event_loggers {
                    loggers.log_execution(EngineEvent::LiveEntry {
                        ts: timestamp,
                        market_slug: market.slug.clone(),
                        side: side_name.to_string(),
                        price: entry_price,
                        size_usd,
                        order_id: Some(order_result.order_id.clone()),
                    });
                }
            }
            Err(err) => {
                eprintln!("[ORDER FAILED] {:?}", err);
            }
        }
        return;
    }

    match signal.exit {
        ExitSignal::FullExit => {
            if !position.is_active() {
                return;
            }

            let (token_id, snapshot_side) = match position.token_side {
                Some(TokenSide::Yes) => (market.yes_token_id, &dual_snapshot.yes),
                Some(TokenSide::No) => (market.no_token_id, &dual_snapshot.no),
                None => return,
            };

            let exit_price = best_bid_price(snapshot_side).unwrap_or(0.0);
            if exit_price < 0.0001 {
                println!("[NO EXIT BID] {:?}", position.token_side);
                return;
            }

            let side_name = match position.token_side {
                Some(TokenSide::Yes) => "YES",
                Some(TokenSide::No) => "NO",
                None => "N/A",
            };

            // Query ACTUAL token balance before selling (don't trust tracked shares)
            let balance_request = BalanceAllowanceRequest::builder()
                .asset_type(AssetType::Conditional)
                .token_id(token_id)
                .build();
            
            let actual_shares = match clob_client.balance_allowance(balance_request).await {
                Ok(b) => {
                    // Balance is in raw units (micro-shares), divide by 10^6
                    const DECIMALS: u32 = 6;
                    let divisor = Decimal::from(10u64.pow(DECIMALS));
                    let human_balance = b.balance / divisor;
                    let bal = decimal_to_f64(human_balance);
                    (bal * 100.0).floor() / 100.0 // Round DOWN to 2 decimals
                }
                Err(_) => position.shares, // Fallback to tracked if query fails
            };

            if actual_shares < 0.01 {
                println!("[NO SHARES] {} | {:?} | Balance too small", market.label, position.token_side);
                position.reset(timestamp, false);
                return;
            }

            let pnl_pct = (exit_price - position.entry_price) / position.entry_price * 100.0;
            let pnl_usd = pnl_pct / 100.0 * (actual_shares * position.entry_price);

            println!(
                "[SIGNAL] {} | SELL {} @ {:.4} | {:.4} shares | PnL: {:.2}% (${:.2})",
                market.label, side_name, exit_price, actual_shares, pnl_pct, pnl_usd
            );

            if dry_run {
                println!("[DRY RUN] Would place market sell order: {} shares of {}", actual_shares, side_name);
                let was_loss = pnl_pct < 0.0;
                gatekeeper.record_trade_result(timestamp, pnl_usd);
                if let Some(loggers) = event_loggers {
                    loggers.log_execution(EngineEvent::LiveExit {
                        ts: timestamp,
                        market_slug: market.slug.clone(),
                        side: side_name.to_string(),
                        price: exit_price,
                        pnl_usd,
                        order_id: None,
                    });
                }
                position.reset(timestamp, was_loss);
                return;
            }

            match place_market_sell(clob_client, signer, token_id, actual_shares).await {
                Ok(order_result) => {
                    let was_loss = pnl_pct < 0.0;
                    gatekeeper.record_trade_result(timestamp, pnl_usd);
                    println!(
                        "[EXIT FILLED] {} | {:.2}% | ${:.2} | Order: {}",
                        market.label, pnl_pct, pnl_usd, order_result.order_id
                    );
                    if let Some(loggers) = event_loggers {
                        loggers.log_execution(EngineEvent::LiveExit {
                            ts: timestamp,
                            market_slug: market.slug.clone(),
                            side: side_name.to_string(),
                            price: exit_price,
                            pnl_usd,
                            order_id: Some(order_result.order_id.clone()),
                        });
                        if gatekeeper.emergency_halt {
                            loggers.log_execution(EngineEvent::EmergencyHalt {
                                ts: timestamp,
                                market_slug: market.slug.clone(),
                                daily_pnl: gatekeeper.daily_pnl,
                                reason: "daily loss limit".to_string(),
                            });
                        }
                    }
                    position.reset(timestamp, was_loss);
                }
                Err(err) => {
                    eprintln!("[EXIT FAILED] {:?} - Position remains open!", err);
                }
            }
        }
        ExitSignal::None => {}
    }
}
