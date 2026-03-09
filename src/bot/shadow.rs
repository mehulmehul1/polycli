use serde::Serialize;

use crate::bot::feed::DualSnapshot;
use crate::bot::risk::{best_ask_price, best_bid_price, trade_allowed};
use crate::bot::signal::{EntrySignal, ExitSignal, SignalState};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum TokenSide {
    Yes,
    No,
}

#[derive(Debug, Serialize)]
pub struct ShadowExitRecord {
    pub side: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl_pct: f64,
    pub pnl_usd: f64,
    pub duration: i64,
    pub bankroll_after: f64,
}

#[derive(Debug, Serialize)]
pub struct ShadowStepResult {
    pub signal_seen: bool,
    pub entry_taken: bool,
    pub entry_blocked: bool,
    pub exit_trade: Option<ShadowExitRecord>,
}

pub struct ShadowPosition {
    pub active_entry: Option<EntrySignal>,
    pub token_side: Option<TokenSide>,
    pub entry_price: f64,
    pub size: f64,
    pub realized_pnl: f64,
    pub position_realized_pnl: f64,
    pub last_exit_timestamp: u64,
    pub entry_timestamp: u64,
    pub position_size_usd: f64,
    pub bankroll_usd: f64,
    pub realized_usd: f64,
    pub position_realized_usd: f64,
    pub yes_blocked: bool,
    pub no_blocked: bool,
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
            bankroll_usd: 5.0,
            realized_usd: 0.0,
            position_realized_usd: 0.0,
            yes_blocked: false,
            no_blocked: false,
        }
    }
}

impl ShadowPosition {
    pub fn is_active(&self) -> bool {
        self.token_side.is_some()
    }

    pub fn reset(&mut self, timestamp: u64) {
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

    pub fn full_reset(&mut self) {
        self.active_entry = None;
        self.token_side = None;
        self.entry_price = 0.0;
        self.size = 0.0;
        self.position_realized_pnl = 0.0;
        self.last_exit_timestamp = 0;
        self.entry_timestamp = 0;
        self.position_size_usd = 0.0;
        self.position_realized_usd = 0.0;
        self.yes_blocked = false;
        self.no_blocked = false;
    }

    pub fn pnl(&self, current_price: f64) -> f64 {
        if !self.is_active() || self.entry_price < 0.0001 {
            return 0.0;
        }
        (current_price - self.entry_price) / self.entry_price
    }
}

pub fn handle_shadow_signals(
    signal: &mut SignalState,
    dual_snapshot: &DualSnapshot,
    shadow: &mut ShadowPosition,
    market_label: &str,
    market_slug: &str,
    market_start_ts: i64,
    market_end_ts: i64,
    timestamp: u64,
    midpoint: f64,
) -> ShadowStepResult {
    let mut result = ShadowStepResult {
        signal_seen: signal.entry != EntrySignal::None,
        entry_taken: false,
        entry_blocked: false,
        exit_trade: None,
    };
    let time_remaining = (market_end_ts - timestamp as i64).max(0);
    let contract_age = (timestamp as i64) - market_start_ts;

    if signal.entry != EntrySignal::None && !shadow.is_active() {
        if !(30..=280).contains(&time_remaining) {
            println!("[FILTER BLOCKED] Time remaining {}s is outside 30s-280s window", time_remaining);
            return result;
        }

        if timestamp.saturating_sub(shadow.last_exit_timestamp) < 15 {
            println!("[FILTER BLOCKED] Cooldown active (last exit was < 15s ago)");
            return result;
        }

        match signal.entry {
            EntrySignal::Long if shadow.yes_blocked => {
                println!("[BLOCKED] YES direction locked after loss");
                result.entry_blocked = true;
                return result;
            }
            EntrySignal::Short if shadow.no_blocked => {
                println!("[BLOCKED] NO direction locked after loss");
                result.entry_blocked = true;
                return result;
            }
            _ => {}
        }

        let position_size_usd = if shadow.position_size_usd > 0.0 {
            shadow.position_size_usd
        } else {
            1.0
        };

        if shadow.bankroll_usd < position_size_usd {
            println!("[BANKROLL BLOCKED] ${:.2}", shadow.bankroll_usd);
            result.entry_blocked = true;
            return result;
        }

        let snapshot_side = match signal.entry {
            EntrySignal::Long => &dual_snapshot.yes,
            EntrySignal::Short => &dual_snapshot.no,
            EntrySignal::None => return result,
        };

        let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(0.0);
        let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(0.0);

        if let Err(reason) = trade_allowed(snapshot_side, time_remaining, contract_age, yes_ask, no_ask) {
            println!(
                "[FILTER BLOCKED ENTRY] {} | {} Side | Reason: {:?}",
                market_slug,
                if matches!(signal.entry, EntrySignal::Long) { "YES" } else { "NO" },
                reason
            );
            result.entry_blocked = true;
            signal.entry = EntrySignal::None;
            return result;
        }

        shadow.token_side = match signal.entry {
            EntrySignal::Long => Some(TokenSide::Yes),
            EntrySignal::Short => Some(TokenSide::No),
            EntrySignal::None => None,
        };

        match best_ask_price(snapshot_side) {
            Some(price) if price > 0.0001 => {
                shadow.active_entry = Some(signal.entry);
                shadow.entry_price = price;
                shadow.size = 1.0;
                shadow.position_realized_pnl = 0.0;
                shadow.entry_timestamp = timestamp;
                shadow.position_size_usd = position_size_usd;
                shadow.bankroll_usd -= position_size_usd;
                shadow.position_realized_usd = 0.0;
                result.entry_taken = true;

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
                    market_label, side_name, price, midpoint, shadow.bankroll_usd
                );
            }
            _ => {
                println!("[NO LIQUIDITY] No best_ask for {:?}", shadow.token_side);
            }
        }

        return result;
    }

    let exit_price = match shadow.token_side {
        Some(TokenSide::Yes) => best_bid_price(&dual_snapshot.yes),
        Some(TokenSide::No) => best_bid_price(&dual_snapshot.no),
        _ => None,
    };

    if signal.exit == ExitSignal::FullExit && shadow.is_active() {
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

                let duration = (timestamp - shadow.entry_timestamp) as i64;
                let side_str = match shadow.token_side {
                    Some(TokenSide::Yes) => "YES".to_string(),
                    Some(TokenSide::No) => "NO".to_string(),
                    None => "N/A".to_string(),
                };
                result.exit_trade = Some(ShadowExitRecord {
                    side: side_str,
                    entry_price: shadow.entry_price,
                    exit_price: price,
                    pnl_pct: shadow.position_realized_pnl,
                    pnl_usd: shadow.position_realized_usd,
                    duration,
                    bankroll_after: shadow.bankroll_usd,
                });

                println!(
                    "[EXIT SLOPE FLIP] {} | {:.4}% | +${:.4} | Bankroll: ${:.2}",
                    market_label,
                    shadow.position_realized_pnl * 100.0,
                    shadow.position_realized_usd,
                    shadow.bankroll_usd
                );
                shadow.reset(timestamp);
            }
            _ => {
                println!("[NO EXIT BID] {:?}", shadow.token_side);
            }
        }
    }

    result
}
