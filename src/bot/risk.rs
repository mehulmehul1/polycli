use crate::bot::feed::MarketSnapshot;
use chrono::{NaiveDate, TimeZone, Utc};
use polymarket_client_sdk::types::Decimal;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterReason {
    NoLiquidity,
    WideSpread,
    ExtremePrice,
    BrokenBook,
    Time,
    Cooldown,
    DailyLossLimit,
    EmergencyHalt,
    DirectionLocked,
    Bankroll,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum GateDecision {
    Approved { reason: String },
    Blocked { reason: FilterReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeDirection {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy)]
pub struct EntryContext {
    pub timestamp: u64,
    pub time_remaining: i64,
    pub min_time_remaining: i64,
    pub max_time_remaining: i64,
    pub contract_age: i64,
    pub yes_ask: f64,
    pub no_ask: f64,
    pub bankroll_available: f64,
    pub position_size_usd: f64,
    pub direction_locked: bool,
    pub direction: TradeDirection,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatekeeperState {
    pub emergency_halt: bool,
    pub daily_loss_limit: f64,
    pub daily_pnl: f64,
    pub trading_date: Option<NaiveDate>,
    pub cooldown_until: Option<u64>,
    pub cooldown_seconds: u64,
}

impl GatekeeperState {
    #[must_use]
    pub fn new(daily_loss_limit: f64, cooldown_seconds: u64) -> Self {
        Self {
            emergency_halt: false,
            daily_loss_limit,
            daily_pnl: 0.0,
            trading_date: None,
            cooldown_until: None,
            cooldown_seconds,
        }
    }

    fn sync_trading_day(&mut self, timestamp: u64) {
        let date = timestamp_to_date(timestamp);
        if self.trading_date != Some(date) {
            self.trading_date = Some(date);
            self.daily_pnl = 0.0;
            self.cooldown_until = None;
        }
    }

    pub fn check_entry(
        &mut self,
        snapshot: &MarketSnapshot,
        context: &EntryContext,
    ) -> GateDecision {
        self.sync_trading_day(context.timestamp);

        if self.emergency_halt {
            return GateDecision::Blocked {
                reason: FilterReason::EmergencyHalt,
            };
        }

        if self.daily_loss_limit > 0.0 && self.daily_pnl <= -self.daily_loss_limit {
            return GateDecision::Blocked {
                reason: FilterReason::DailyLossLimit,
            };
        }

        if let Some(cooldown_until) = self.cooldown_until {
            if context.timestamp < cooldown_until {
                return GateDecision::Blocked {
                    reason: FilterReason::Cooldown,
                };
            }
        }

        if context.direction_locked {
            return GateDecision::Blocked {
                reason: FilterReason::DirectionLocked,
            };
        }

        if context.bankroll_available + f64::EPSILON < context.position_size_usd {
            return GateDecision::Blocked {
                reason: FilterReason::Bankroll,
            };
        }

        if context.time_remaining < context.min_time_remaining
            || context.time_remaining > context.max_time_remaining
        {
            return GateDecision::Blocked {
                reason: FilterReason::Time,
            };
        }

        match trade_allowed(
            snapshot,
            context.time_remaining,
            context.contract_age,
            context.yes_ask,
            context.no_ask,
        ) {
            Ok(()) => GateDecision::Approved {
                reason: format!(
                    "{:?} entry approved (daily_pnl={:.2}, cooldown_until={:?})",
                    context.direction, self.daily_pnl, self.cooldown_until
                ),
            },
            Err(reason) => GateDecision::Blocked { reason },
        }
    }

    pub fn record_trade_result(&mut self, timestamp: u64, pnl_usd: f64) {
        self.sync_trading_day(timestamp);
        self.daily_pnl += pnl_usd;

        if pnl_usd < 0.0 {
            self.cooldown_until = Some(timestamp + self.cooldown_seconds);
        }

        if self.daily_loss_limit > 0.0 && self.daily_pnl <= -self.daily_loss_limit {
            self.emergency_halt = true;
        }
    }

    pub fn halt(&mut self) {
        self.emergency_halt = true;
    }

    pub fn clear_halt(&mut self) {
        self.emergency_halt = false;
    }
}

impl Default for GatekeeperState {
    fn default() -> Self {
        Self::new(5.0, 15)
    }
}

pub fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_string().parse::<f64>().unwrap_or_default()
}

pub fn best_ask_price(snapshot: &MarketSnapshot) -> Option<f64> {
    snapshot.best_ask.map(decimal_to_f64)
}

pub fn best_bid_price(snapshot: &MarketSnapshot) -> Option<f64> {
    snapshot.best_bid.map(decimal_to_f64)
}

pub fn midpoint_price(snapshot: &MarketSnapshot) -> Option<f64> {
    if let Some(mid) = snapshot.midpoint {
        return Some(decimal_to_f64(mid));
    }

    match (snapshot.best_bid, snapshot.best_ask) {
        (Some(bid), Some(ask)) => Some((decimal_to_f64(bid) + decimal_to_f64(ask)) / 2.0),
        _ => None,
    }
}

pub fn trade_allowed(
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

    let bid = best_bid.unwrap_or_default();
    let ask = best_ask.unwrap_or_default();

    let spread = ask - bid;
    let max_spread = (ask * 0.10).max(0.03);
    if spread > max_spread {
        return Err(FilterReason::WideSpread);
    }

    if !(0.35..=0.65).contains(&ask) {
        return Err(FilterReason::ExtremePrice);
    }

    if (yes_ask + no_ask - 1.0).abs() > 0.10 {
        return Err(FilterReason::BrokenBook);
    }

    if time_remaining < 30 || contract_age < 15 {
        return Err(FilterReason::Time);
    }

    Ok(())
}

fn timestamp_to_date(timestamp: u64) -> NaiveDate {
    Utc.timestamp_opt(timestamp as i64, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(best_bid: f64, best_ask: f64) -> MarketSnapshot {
        MarketSnapshot {
            midpoint: Decimal::from_f64_retain((best_bid + best_ask) / 2.0),
            best_bid: Decimal::from_f64_retain(best_bid),
            best_ask: Decimal::from_f64_retain(best_ask),
            spread: Decimal::from_f64_retain(best_ask - best_bid),
            top5_bid_depth: Decimal::new(50000, 2),
            top5_ask_depth: Decimal::new(50000, 2),
        }
    }

    fn entry_context(timestamp: u64) -> EntryContext {
        EntryContext {
            timestamp,
            time_remaining: 60,
            min_time_remaining: 30,
            max_time_remaining: 280,
            contract_age: 30,
            yes_ask: 0.50,
            no_ask: 0.50,
            bankroll_available: 10.0,
            position_size_usd: 1.0,
            direction_locked: false,
            direction: TradeDirection::Yes,
        }
    }

    #[test]
    fn trade_allowed_passes_good_conditions() {
        assert!(trade_allowed(&snapshot(0.47, 0.50), 60, 30, 0.50, 0.50).is_ok());
    }

    #[test]
    fn trade_allowed_blocks_wide_spread() {
        assert_eq!(
            trade_allowed(&snapshot(0.40, 0.60), 60, 30, 0.60, 0.40),
            Err(FilterReason::WideSpread)
        );
    }

    #[test]
    fn trade_allowed_blocks_extreme_price() {
        assert_eq!(
            trade_allowed(&snapshot(0.84, 0.86), 60, 30, 0.86, 0.14),
            Err(FilterReason::ExtremePrice)
        );
    }

    #[test]
    fn trade_allowed_blocks_broken_book() {
        assert_eq!(
            trade_allowed(&snapshot(0.49, 0.51), 60, 30, 0.99, 0.99),
            Err(FilterReason::BrokenBook)
        );
    }

    #[test]
    fn gatekeeper_allows_normal_entry() {
        let mut gatekeeper = GatekeeperState::new(5.0, 15);
        let decision = gatekeeper.check_entry(&snapshot(0.47, 0.50), &entry_context(1_700_000_000));
        assert!(matches!(decision, GateDecision::Approved { .. }));
    }

    #[test]
    fn gatekeeper_blocks_after_daily_loss_limit() {
        let mut gatekeeper = GatekeeperState::new(2.0, 15);
        gatekeeper.record_trade_result(1_700_000_000, -2.25);

        let decision = gatekeeper.check_entry(&snapshot(0.47, 0.50), &entry_context(1_700_000_030));
        assert!(matches!(
            decision,
            GateDecision::Blocked {
                reason: FilterReason::EmergencyHalt | FilterReason::DailyLossLimit
            }
        ));
    }

    #[test]
    fn gatekeeper_emergency_halt_blocks_all() {
        let mut gatekeeper = GatekeeperState::new(5.0, 15);
        gatekeeper.halt();

        let decision = gatekeeper.check_entry(&snapshot(0.47, 0.50), &entry_context(1_700_000_000));
        assert!(matches!(
            decision,
            GateDecision::Blocked {
                reason: FilterReason::EmergencyHalt
            }
        ));
    }

    #[test]
    fn gatekeeper_cooldown_blocks_then_expires() {
        let mut gatekeeper = GatekeeperState::new(5.0, 15);
        gatekeeper.record_trade_result(1_700_000_000, -0.50);

        let blocked = gatekeeper.check_entry(&snapshot(0.47, 0.50), &entry_context(1_700_000_010));
        assert!(matches!(
            blocked,
            GateDecision::Blocked {
                reason: FilterReason::Cooldown
            }
        ));

        let approved = gatekeeper.check_entry(&snapshot(0.47, 0.50), &entry_context(1_700_000_020));
        assert!(matches!(approved, GateDecision::Approved { .. }));
    }
}
