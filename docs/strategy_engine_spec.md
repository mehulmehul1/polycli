# Strategy Engine Specification

## 1. Goal

Replace the current fragmented strategy logic in `signal.rs` and `strategy_runner.rs` with a **unified `StrategyEngine` API** that:

1. Takes a single observation snapshot per tick
2. Returns a structured `StrategyDecision` (entry, exit, hold, block)
3. Fuses multiple signal sources (indicators, book inefficiency, Qlib scores)
4. Maintains deterministic state for replay and backtest
5. Separates decision logic from execution plumbing

## 2. Current State (Pre-Refactor)

### 2.1 Fragmented logic locations

| File | Responsibility | Problem |
|------|---------------|---------|
| `signal.rs` | Indicator-based entry/exit | Couples to `IndicatorState`, uses midpoint probability |
| `strategy_runner.rs` | Orchestrates candles, indicators, book check | Mixes signal generation with shadow position tracking |
| `risk.rs` | Gatekeeper entry filters | Independent of signal confidence |
| `shadow.rs` | Position simulation | Doesn't know *why* entry was triggered |

### 2.2 Current signal sources

```rust
// From strategy_runner.rs (lines 50-67)
let indicator_signal = signal_engine.update(state_5s, state_1m, midpoint).entry;
let book_signal = if book_sum > 1.03 {
    EntrySignal::Short
} else if book_sum < 0.97 {
    EntrySignal::Long
} else {
    EntrySignal::None
};

// Priority: indicators > book inefficiency
signal.entry = if indicator_signal != EntrySignal::None {
    indicator_signal
} else if book_signal != EntrySignal::None {
    book_signal
} else {
    EntrySignal::None
};
```

### 2.3 Problems with current approach

1. **No confidence score** — Binary Long/Short/None, no gradation
2. **No fair-value reference** — Decisions based on token price patterns, not mispricing vs fundamental value
3. **Hard-coded priority** — Indicators always win over book signals
4. **No Qlib integration path** — Nowhere to inject model scores
5. **Exit logic disconnected** — Exit only checks momentum, ignores why we entered
6. **No entry reason tracking** — Can't analyze which signal source performed best

## 3. Target Architecture

### 3.1 Core types

```rust
// src/bot/strategy/types.rs

/// Direction for YES/NO markets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Yes,
    No,
}

/// Signal source identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalSource {
    /// Indicator-based (EMA crossover, RSI, BB expansion)
    Indicators,
    /// Book inefficiency (yes_ask + no_ask deviation from 1.0)
    BookInefficiency,
    /// Fair-value mispricing (model-based probability vs market price)
    FairValue,
    /// Qlib model score above threshold
    QlibScore,
    /// Fused combination of multiple sources
    Fused,
}

/// Confidence level for entry decision
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence(f64);

impl Confidence {
    pub const MIN: Self = Self(0.0);
    pub const LOW: Self = Self(0.25);
    pub const MEDIUM: Self = Self(0.50);
    pub const HIGH: Self = Self(0.75);
    pub const MAX: Self = Self(1.0);

    pub fn new(value: f64) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    pub fn value(&self) -> f64 {
        self.0
    }

    pub fn above_threshold(&self, threshold: f64) -> bool {
        self.0 >= threshold
    }
}

/// Why an entry was triggered
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryReason {
    pub source: SignalSource,
    pub confidence: Confidence,
    pub detail: String,
    pub fair_value_edge: Option<f64>,
    pub qlib_score: Option<f64>,
}

/// Why an exit was triggered
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExitReason {
    /// Momentum reversal (EMA crossover or slope flip)
    MomentumReversal,
    /// Take profit threshold reached
    TakeProfit { pnl_pct: f64 },
    /// Stop loss threshold reached
    StopLoss { pnl_pct: f64 },
    /// Time-based exit (market ending)
    TimeExpiry { seconds_remaining: i64 },
    /// Risk gate triggered (spread too wide, book broken)
    RiskGate { reason: String },
    /// Position held too long
    MaxHoldingTime { seconds_held: u64 },
    /// Fair-value model now disagrees with position
    FairValueReversal,
    /// Qlib score dropped below threshold
    QlibScoreDrop,
}

/// Strategy decision output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyDecision {
    /// No action
    Hold,
    /// Block all entries (risk gate, cooldown, etc.)
    Block { reason: String },
    /// Enter a new position
    Enter {
        direction: Direction,
        reason: EntryReason,
        suggested_size_usd: f64,
    },
    /// Exit current position
    Exit {
        reason: ExitReason,
        pnl_estimate: f64,
    },
}

/// Observation input to strategy engine
#[derive(Debug, Clone)]
pub struct StrategyObservation {
    // Timestamp
    pub ts: u64,
    
    // Market identity
    pub condition_id: String,
    pub market_slug: String,
    pub asset: String,
    pub duration: String,
    pub market_start_ts: i64,
    pub market_end_ts: i64,
    
    // Book state
    pub yes_bid: f64,
    pub yes_ask: f64,
    pub no_bid: f64,
    pub no_ask: f64,
    pub yes_spread: f64,
    pub no_spread: f64,
    pub book_sum: f64,
    pub book_gap: f64,
    
    // Derived
    pub yes_mid: f64,
    pub no_mid: f64,
    
    // Indicator state (from existing IndicatorEngine)
    pub ind_5s: IndicatorState,
    pub ind_1m: IndicatorState,
    
    // Fair value (optional, from reference price model)
    pub fair_prob_yes: Option<f64>,
    pub fair_prob_calibrated: Option<f64>,
    
    // Qlib scores (optional, from score_loader)
    pub qlib_score_yes: Option<f64>,
    pub qlib_score_no: Option<f64>,
    pub qlib_fresh: bool,
    
    // Current position state
    pub position: Option<ActivePosition>,
    
    // Risk state
    pub risk_gates: RiskGateState,
}

/// Active position being tracked
#[derive(Debug, Clone)]
pub struct ActivePosition {
    pub direction: Direction,
    pub entry_ts: u64,
    pub entry_price: f64,
    pub size_usd: f64,
    pub entry_reason: EntryReason,
}

/// Risk gate state snapshot
#[derive(Debug, Clone)]
pub struct RiskGateState {
    pub spread_ok: bool,
    pub price_range_ok: bool,
    pub book_integrity_ok: bool,
    pub time_remaining_ok: bool,
    pub bankroll_ok: bool,
    pub cooldown_ok: bool,
    pub daily_loss_ok: bool,
    pub direction_unlocked: bool,
    pub feed_stale: bool,
}
```

### 3.2 StrategyEngine trait

```rust
// src/bot/strategy/engine.rs

/// Core strategy engine trait
///
/// Implementations:
/// - `HeuristicEngine` — current indicator + book logic
/// - `QlibFusedEngine` — heuristic + Qlib score fusion
/// - `BacktestEngine` — replay with historical scores
pub trait StrategyEngine: Send + Sync {
    /// Process observation and return decision
    fn on_observation(&mut self, obs: &StrategyObservation) -> StrategyDecision;
    
    /// Reset state for new market
    fn reset(&mut self);
    
    /// Get engine name for logging
    fn name(&self) -> &'static str;
    
    /// Get current internal state (for debugging/replay)
    fn state_summary(&self) -> serde_json::Value;
}
```

### 3.3 HeuristicEngine implementation

```rust
// src/bot/strategy/heuristic.rs

/// Heuristic strategy engine (current logic refactored)
pub struct HeuristicEngine {
    // Configuration
    config: HeuristicConfig,
    
    // Internal state
    recent_prices: VecDeque<f64>,
    active_position: Option<ActivePosition>,
    
    // Signal engines (reused from current code)
    indicator_checker: IndicatorSignalChecker,
    book_checker: BookInefficiencyChecker,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeuristicConfig {
    // Entry band (price range where entries allowed)
    pub entry_band_low: f64,
    pub entry_band_high: f64,
    
    // Indicator thresholds
    pub bb_width_min: f64,
    pub bb_width_max: f64,
    pub slope_threshold: f64,
    pub rsi_oversold: f64,
    pub rsi_overbought: f64,
    
    // Book inefficiency thresholds
    pub book_sum_long_threshold: f64,  // < this = long YES
    pub book_sum_short_threshold: f64, // > this = short YES
    
    // Signal priority
    pub prefer_indicators: bool,
    
    // Exit thresholds
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub max_holding_seconds: u64,
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        Self {
            entry_band_low: 0.08,
            entry_band_high: 0.92,
            bb_width_min: 0.15,
            bb_width_max: 0.90,
            slope_threshold: 0.002,
            rsi_oversold: 30.0,
            rsi_overbought: 70.0,
            book_sum_long_threshold: 0.97,
            book_sum_short_threshold: 1.03,
            prefer_indicators: true,
            take_profit_pct: 0.20,
            stop_loss_pct: -0.15,
            max_holding_seconds: 120,
        }
    }
}

impl StrategyEngine for HeuristicEngine {
    fn on_observation(&mut self, obs: &StrategyObservation) -> StrategyDecision {
        // 1. Check risk gates
        if !obs.risk_gates.all_ok() {
            return StrategyDecision::Block {
                reason: obs.risk_gates.block_reason(),
            };
        }
        
        // 2. If in position, check exit first
        if let Some(ref pos) = obs.position {
            if let Some(exit) = self.check_exit(obs, pos) {
                return exit;
            }
        }
        
        // 3. If not in position, check entry
        if obs.position.is_none() {
            if let Some(entry) = self.check_entry(obs) {
                return entry;
            }
        }
        
        StrategyDecision::Hold
    }
    
    fn reset(&mut self) {
        self.recent_prices.clear();
        self.active_position = None;
    }
    
    fn name(&self) -> &'static str {
        "heuristic"
    }
    
    fn state_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "recent_prices_count": self.recent_prices.len(),
            "has_position": self.active_position.is_some(),
        })
    }
}

impl HeuristicEngine {
    fn check_entry(&mut self, obs: &StrategyObservation) -> Option<StrategyDecision> {
        // Price range filter
        if obs.yes_mid < self.config.entry_band_low 
            || obs.yes_mid > self.config.entry_band_high {
            return None;
        }
        
        // Gather signals
        let indicator_signal = self.indicator_checker.check(&obs.ind_5s, &obs.ind_1m, obs.yes_mid);
        let book_signal = self.book_checker.check(obs.book_sum);
        
        // Combine signals
        let (direction, reason) = if self.config.prefer_indicators {
            if indicator_signal.is_some() {
                indicator_signal
            } else {
                book_signal
            }
        } else {
            // Priority by confidence
            match (indicator_signal, book_signal) {
                (Some((d1, r1)), Some((d2, r2))) => {
                    if r1.confidence >= r2.confidence {
                        Some((d1, r1))
                    } else {
                        Some((d2, r2))
                    }
                }
                (Some(sig), None) => Some(sig),
                (None, Some(sig)) => Some(sig),
                (None, None) => None,
            }
        }?;
        
        Some(StrategyDecision::Enter {
            direction,
            reason,
            suggested_size_usd: 5.0, // TODO: from config/bankroll
        })
    }
    
    fn check_exit(&mut self, obs: &StrategyObservation, pos: &ActivePosition) -> Option<StrategyDecision> {
        let current_price = match pos.direction {
            Direction::Yes => obs.yes_bid,
            Direction::No => obs.no_bid,
        };
        
        let pnl_pct = (current_price - pos.entry_price) / pos.entry_price;
        let time_held = obs.ts - pos.entry_ts;
        
        // Take profit
        if pnl_pct >= self.config.take_profit_pct {
            return Some(StrategyDecision::Exit {
                reason: ExitReason::TakeProfit { pnl_pct },
                pnl_estimate: pnl_pct * pos.size_usd,
            });
        }
        
        // Stop loss
        if pnl_pct <= self.config.stop_loss_pct {
            return Some(StrategyDecision::Exit {
                reason: ExitReason::StopLoss { pnl_pct },
                pnl_estimate: pnl_pct * pos.size_usd,
            });
        }
        
        // Max holding time
        if time_held >= self.config.max_holding_seconds {
            return Some(StrategyDecision::Exit {
                reason: ExitReason::MaxHoldingTime { seconds_held: time_held },
                pnl_estimate: pnl_pct * pos.size_usd,
            });
        }
        
        // Momentum reversal
        let momentum_flipped = match pos.direction {
            Direction::Yes => {
                obs.ind_5s.ema3 < obs.ind_5s.ema6 || obs.ind_5s.momentum_slope.unwrap_or(0.0) < 0.0
            }
            Direction::No => {
                obs.ind_5s.ema3 > obs.ind_5s.ema6 || obs.ind_5s.momentum_slope.unwrap_or(0.0) > 0.0
            }
        };
        
        if momentum_flipped {
            return Some(StrategyDecision::Exit {
                reason: ExitReason::MomentumReversal,
                pnl_estimate: pnl_pct * pos.size_usd,
            });
        }
        
        None
    }
}
```

### 3.4 QlibFusedEngine implementation

```rust
// src/bot/strategy/qlib_fused.rs

/// Fused strategy engine combining heuristics with Qlib scores
pub struct QlibFusedEngine {
    heuristic: HeuristicEngine,
    config: FusionConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FusionConfig {
    /// Minimum Qlib score to trigger entry
    pub qlib_score_threshold: f64,
    
    /// Confidence boost when Qlib agrees with heuristic
    pub qlib_agreement_boost: f64,
    
    /// Require Qlib score to be fresh (within TTL)
    pub qlib_freshness_required: bool,
    
    /// Block entry if Qlib disagrees and confidence > threshold
    pub qlib_veto_enabled: bool,
    pub qlib_veto_confidence: f64,
    
    /// Use fair-value edge as primary signal
    pub fair_value_primary: bool,
    pub fair_value_edge_threshold: f64,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            qlib_score_threshold: 0.60,
            qlib_agreement_boost: 0.20,
            qlib_freshness_required: true,
            qlib_veto_enabled: true,
            qlib_veto_confidence: 0.70,
            fair_value_primary: false,
            fair_value_edge_threshold: 0.03,
        }
    }
}

impl StrategyEngine for QlibFusedEngine {
    fn on_observation(&mut self, obs: &StrategyObservation) -> StrategyDecision {
        // 1. Check risk gates
        if !obs.risk_gates.all_ok() {
            return StrategyDecision::Block {
                reason: obs.risk_gates.block_reason(),
            };
        }
        
        // 2. Get heuristic decision
        let heuristic_decision = self.heuristic.on_observation(obs);
        
        // 3. Check Qlib score validity
        if self.config.qlib_freshness_required && !obs.qlib_fresh {
            // Degrade to heuristic-only if Qlib stale
            return heuristic_decision;
        }
        
        // 4. Fuse decisions
        match &heuristic_decision {
            StrategyDecision::Enter { direction, reason, suggested_size_usd } => {
                self.fuse_entry(obs, *direction, reason.clone(), *suggested_size_usd)
            }
            StrategyDecision::Exit { reason, pnl_estimate } => {
                self.fuse_exit(obs, reason, *pnl_estimate)
            }
            StrategyDecision::Hold => {
                // Check if Qlib alone triggers entry
                self.check_qlib_only_entry(obs)
            }
            StrategyDecision::Block { reason } => heuristic_decision,
        }
    }
    
    fn reset(&mut self) {
        self.heuristic.reset();
    }
    
    fn name(&self) -> &'static str {
        "qlib_fused"
    }
    
    fn state_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "heuristic": self.heuristic.state_summary(),
            "config": self.config,
        })
    }
}

impl QlibFusedEngine {
    fn fuse_entry(
        &self,
        obs: &StrategyObservation,
        direction: Direction,
        mut reason: EntryReason,
        size: f64,
    ) -> StrategyDecision {
        let qlib_score = match direction {
            Direction::Yes => obs.qlib_score_yes,
            Direction::No => obs.qlib_score_no,
        };
        
        match qlib_score {
            Some(score) if score >= self.config.qlib_score_threshold => {
                // Qlib agrees — boost confidence
                reason.source = SignalSource::Fused;
                reason.qlib_score = Some(score);
                reason.confidence = Confidence::new(
                    reason.confidence.value() + self.config.qlib_agreement_boost
                );
                StrategyDecision::Enter {
                    direction,
                    reason,
                    suggested_size_usd: size,
                }
            }
            Some(score) if self.config.qlib_veto_enabled 
                && score < (1.0 - self.config.qlib_veto_confidence) => {
                // Qlib strongly disagrees — veto entry
                StrategyDecision::Block {
                    reason: format!(
                        "qlib_veto: score={:.3} < {:.3} for {:?}",
                        score,
                        1.0 - self.config.qlib_veto_confidence,
                        direction
                    ),
                }
            }
            _ => {
                // Qlib neutral or missing — use heuristic
                StrategyDecision::Enter {
                    direction,
                    reason,
                    suggested_size_usd: size,
                }
            }
        }
    }
    
    fn fuse_exit(
        &self,
        obs: &StrategyObservation,
        reason: &ExitReason,
        pnl: f64,
    ) -> StrategyDecision {
        // Check if Qlib score suggests early exit
        if let Some(ref pos) = obs.position {
            let qlib_score = match pos.direction {
                Direction::Yes => obs.qlib_score_yes,
                Direction::No => obs.qlib_score_no,
            };
            
            if let Some(score) = qlib_score {
                if score < 0.40 {
                    return StrategyDecision::Exit {
                        reason: ExitReason::QlibScoreDrop,
                        pnl_estimate: pnl,
                    };
                }
            }
        }
        
        StrategyDecision::Exit {
            reason: reason.clone(),
            pnl_estimate: pnl,
        }
    }
    
    fn check_qlib_only_entry(&self, obs: &StrategyObservation) -> StrategyDecision {
        // Only allow Qlib-only entry if score is very high
        let yes_score = obs.qlib_score_yes.unwrap_or(0.0);
        let no_score = obs.qlib_score_no.unwrap_or(0.0);
        
        if yes_score >= 0.80 {
            return StrategyDecision::Enter {
                direction: Direction::Yes,
                reason: EntryReason {
                    source: SignalSource::QlibScore,
                    confidence: Confidence::new(yes_score),
                    detail: format!("qlib_only: score={:.3}", yes_score),
                    fair_value_edge: None,
                    qlib_score: Some(yes_score),
                },
                suggested_size_usd: 5.0,
            };
        }
        
        if no_score >= 0.80 {
            return StrategyDecision::Enter {
                direction: Direction::No,
                reason: EntryReason {
                    source: SignalSource::QlibScore,
                    confidence: Confidence::new(no_score),
                    detail: format!("qlib_only: score={:.3}", no_score),
                    fair_value_edge: None,
                    qlib_score: Some(no_score),
                },
                suggested_size_usd: 5.0,
            };
        }
        
        StrategyDecision::Hold
    }
}
```

### 3.5 RiskGateState helper

```rust
// src/bot/strategy/risk_gates.rs

impl RiskGateState {
    pub fn all_ok(&self) -> bool {
        self.spread_ok
            && self.price_range_ok
            && self.book_integrity_ok
            && self.time_remaining_ok
            && self.bankroll_ok
            && self.cooldown_ok
            && self.daily_loss_ok
            && self.direction_unlocked
            && !self.feed_stale
    }
    
    pub fn block_reason(&self) -> String {
        if self.feed_stale {
            return "feed_stale".to_string();
        }
        if !self.spread_ok {
            return "wide_spread".to_string();
        }
        if !self.price_range_ok {
            return "extreme_price".to_string();
        }
        if !self.book_integrity_ok {
            return "broken_book".to_string();
        }
        if !self.time_remaining_ok {
            return "time_window".to_string();
        }
        if !self.bankroll_ok {
            return "insufficient_bankroll".to_string();
        }
        if !self.cooldown_ok {
            return "cooldown".to_string();
        }
        if !self.daily_loss_ok {
            return "daily_loss_limit".to_string();
        }
        if !self.direction_unlocked {
            return "direction_locked".to_string();
        }
        "unknown".to_string()
    }
}
```

## 4. Integration Points

### 4.1 Module structure

```
src/bot/strategy/
  mod.rs           — exports StrategyEngine trait
  types.rs         — StrategyDecision, Direction, Confidence, etc.
  engine.rs        — StrategyEngine trait definition
  heuristic.rs     — HeuristicEngine implementation
  qlib_fused.rs    — QlibFusedEngine implementation
  risk_gates.rs    — RiskGateState helpers
  indicator_checker.rs — extracts indicator logic from signal.rs
  book_checker.rs  — extracts book inefficiency logic
```

### 4.2 CLI integration

```bash
# Run with heuristic engine (current behavior)
polymarket bot watch-btc --engine heuristic

# Run with Qlib-fused engine
polymarket bot watch-btc --engine fused --scores /path/to/scores.parquet

# Run backtest comparing engines
polymarket bot backtest \
  --input pmxt.parquet \
  --engine heuristic \
  --engine fused \
  --scores /path/to/scores.parquet \
  --compare
```

### 4.3 Logging format

```json
{
  "ts": 1710123456,
  "market_slug": "btc-updown-5m-1772243100",
  "decision": "enter",
  "direction": "yes",
  "source": "fused",
  "confidence": 0.85,
  "qlib_score": 0.72,
  "fair_value_edge": 0.04,
  "detail": "indicator=Long, book=Long, qlib=0.72"
}
```

## 5. Migration Path

### Phase 1: Extract types

1. Create `src/bot/strategy/types.rs` with all types
2. Update `signal.rs` to use `Direction` instead of `EntrySignal`
3. Add `EntryReason` tracking to shadow positions

### Phase 2: Create trait and heuristic

1. Create `StrategyEngine` trait
2. Implement `HeuristicEngine` by extracting logic from `strategy_runner.rs`
3. Add unit tests matching current behavior

### Phase 3: Wire to CLI

1. Add `--engine` flag to `watch-btc` and `trade-btc`
2. Default to `heuristic` (no behavior change)
3. Add `StrategyDecision` logging

### Phase 4: Add Qlib fusion

1. Implement `QlibFusedEngine`
2. Add `--scores` flag to CLI
3. Run side-by-side comparison in shadow mode

### Phase 5: Deprecate old code

1. Mark `signal.rs` and `strategy_runner.rs` as deprecated
2. Update all callers to use `StrategyEngine`
3. Remove old code after validation

## 6. Test Plan

### 6.1 Unit tests

- `HeuristicEngine` matches current `signal.rs` behavior for same inputs
- `QlibFusedEngine` boosts confidence when Qlib agrees
- `QlibFusedEngine` vetoes when Qlib strongly disagrees
- Risk gates block correctly
- Exit triggers fire at correct thresholds

### 6.2 Integration tests

- Replay with `heuristic` produces same trades as current `strategy_runner.rs`
- Replay with `fused` loads scores and produces different decisions
- Missing scores degrade gracefully to heuristic
- Stale scores are ignored when `qlib_freshness_required=true`

### 6.3 Acceptance criteria

- [ ] All existing tests pass with new engine
- [ ] `heuristic` engine produces identical replay results to current code
- [ ] `fused` engine logs entry reasons with Qlib scores
- [ ] `fused` engine blocks entry when Qlib score < 0.40 and heuristic triggered
- [ ] `fused` engine exits early when Qlib score drops below 0.40
- [ ] Risk gates are checked before any entry decision
