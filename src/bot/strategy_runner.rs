use crate::bot::candles::CandleEngine;
use crate::bot::feed::DualSnapshot;
use crate::bot::indicators::{IndicatorEngine, IndicatorState};
use crate::bot::risk::{best_ask_price, decimal_to_f64, midpoint_price};
use crate::bot::shadow::{handle_shadow_signals, ShadowPosition, ShadowStepResult};
use crate::bot::signal::{EntrySignal, SignalEngine};

#[allow(clippy::too_many_arguments)]
pub fn run_shadow_strategy_step(
    dual_snapshot: &DualSnapshot,
    market_label: &str,
    market_slug: &str,
    market_start_ts: i64,
    market_end_ts: i64,
    epoch_seconds: u64,
    position_size_usd: f64,
    candle_engine: &mut CandleEngine,
    ind_1m: &mut IndicatorEngine,
    ind_5s: &mut IndicatorEngine,
    signal_engine: &mut SignalEngine,
    state_1m: &mut IndicatorState,
    state_5s: &mut IndicatorState,
    shadow: &mut ShadowPosition,
) -> Option<ShadowStepResult> {
    let midpoint = midpoint_price(&dual_snapshot.yes)?;
    let simulated_volume =
        decimal_to_f64(dual_snapshot.yes.top5_bid_depth + dual_snapshot.yes.top5_ask_depth);
    let spread_f64 = dual_snapshot.yes.spread.map(decimal_to_f64).unwrap_or(0.0);

    let time_remaining = market_end_ts - (epoch_seconds as i64);

    if spread_f64 > 0.08 {
        if epoch_seconds % 30 == 0 {
            println!("[DEBUG] Blocked by SPREAD: {:.4} > 0.08", spread_f64);
        }
        return None;
    }

    if time_remaining < 45 {
        return None;
    }

    if midpoint > 0.92 || midpoint < 0.08 {
        if epoch_seconds % 30 == 0 {
            println!("[DEBUG] Blocked by RANGE: {:.4} (Market trending to end)", midpoint);
        }
        return None;
    }

    let closed_candles =
        candle_engine.update(midpoint, spread_f64, simulated_volume, epoch_seconds)?;

    if let Some(c) = closed_candles.one_minute {
        *state_1m = ind_1m.update(&c);
    }

    if let Some(c) = closed_candles.five_second {
        *state_5s = ind_5s.update(&c);

        let mut signal = signal_engine.update(state_5s, state_1m, midpoint);

        // Orderbook Inefficiency Detection
        let yes_ask = best_ask_price(&dual_snapshot.yes).unwrap_or(1.0);
        let no_ask = best_ask_price(&dual_snapshot.no).unwrap_or(1.0);
        let book_sum = yes_ask + no_ask;
        let book_signal = if book_sum > 1.03 {
            EntrySignal::Short // YES overpriced
        } else if book_sum < 0.97 {
            EntrySignal::Long // NO overpriced
        } else {
            EntrySignal::None
        };

        if book_sum > 1.03 || book_sum < 0.97 {
            println!("[SIGNAL] BOOK_INEFFICIENCY: sum={:.4} signal={:?}", book_sum, book_signal);
        }

        let indicator_signal = signal.entry;
        if indicator_signal != EntrySignal::None {
            let bbw = state_5s.bb_width.unwrap_or(0.0);
            let bbp = state_5s.bb_percent.unwrap_or(0.0);
            println!("[SIGNAL] INDICATOR_MATCH: {:?} (BBW={:.4} BBP={:.4})", indicator_signal, bbw, bbp);
        }

        // Final Logic
        // Final Logic: Signal if indicators confirmed OR book inefficiency detected
        signal.entry = if indicator_signal != EntrySignal::None {
            println!("[SIGNAL] FINAL: Using Indicators ({:?})", indicator_signal);
            indicator_signal
        } else if book_signal != EntrySignal::None {
            println!("[SIGNAL] FINAL: Using Book Inefficiency ({:?})", book_signal);
            book_signal
        } else {
            EntrySignal::None
        };

        shadow.position_size_usd = position_size_usd;
        return Some(handle_shadow_signals(
            &mut signal,
            dual_snapshot,
            shadow,
            market_label,
            market_slug,
            market_start_ts,
            market_end_ts,
            epoch_seconds,
            midpoint,
        ));
    }

    None
}
