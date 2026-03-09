use crate::bot::feed::MarketSnapshot;
use polymarket_client_sdk::types::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterReason {
    NoLiquidity,
    WideSpread,
    ExtremePrice,
    BrokenBook,
    Time,
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
