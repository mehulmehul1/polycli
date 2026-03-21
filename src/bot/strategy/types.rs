//! Strategy Engine Types
//!
//! Core types for trading decisions.

use serde::{Deserialize, Serialize};

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

impl Default for Confidence {
    fn default() -> Self {
        Self::MEDIUM
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
    },
    /// Exit current position
    Exit {
        position_id: String,
        reason: ExitReason,
    },
}

impl Default for StrategyDecision {
    fn default() -> Self {
        Self::Hold
    }
}

/// Fusion mode for combining signals
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FusionMode {
    #[default]
    HeuristicOnly,
    QlibOnly,
    Fused,
}

/// Entry signal (legacy compatibility with signal.rs)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySignal {
    Long,
    Short,
    None,
}

/// Exit signal (legacy compatibility with signal.rs)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitSignal {
    FullExit,
    None,
}

/// Strategy mode for market classifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyMode {
    Scalper,
    FairValue,
    LateWindow,
    TemporalArb,
}
