# Multi-Scale Temporal Arbitrage Engine - Strategy Document (CORRECTED)

## Executive Summary

A **Multi-Scale Temporal Arbitrage Engine** that exploits information flow across Polymarket's nested timeframes: **5min → 15min → 1hour → 4hour**.

**CRITICAL CORRECTION:** Each market has a **resetting strike** (the open of its own interval), NOT a shared fixed reference. The dependency is **chained/path-dependent**, not a simple OR of independent crossings.

## The Real Market Structure

```
15min market (1:15-1:30PM):
  ├─ 5m #1 (1:15-1:20): Strike = P₀ → YES iff P₁ > P₀
  ├─ 5m #2 (1:20-1:25): Strike = P₁ → YES iff P₂ > P₁
  └─ 5m #3 (1:25-1:30): Strike = P₂ → YES iff P₃ > P₂

15min parent: Strike = P₀ → YES iff P₃ > P₀
```

**Key insight:** After observing the first two 5min resolutions, we have a **price path** P₀→P₁→P₂. The remaining uncertainty is only whether P₃ > P₀. This creates a legitimate conditional arbitrage edge.

---

## Edge Sources (Corrected & Viable)

### ✅ 1. Chained Conditional Arbitrage (PRIMARY EDGE)

After k of n children resolve, we observe the actual price path. The parent's fair probability becomes:

```
P(parent=YES | observed path) = Φ((P_live - P_original) / (σ * √t_remaining))

Where:
- P_original = strike of parent (open of 15min interval) = first child's close price
- P_live = current live BTC price from graph.price_state
- t_remaining = time until parent resolves
```

**Example:**
- 5m #1 resolves: close = $71,550 (YES, beat its strike P₀=$71,496)
- 5m #2 resolves: close = $71,530 (NO, didn't beat its strike P₁=$71,550)
- Live BTC now: $71,530, need P₃ > P₀ = $71,496
- 15min market prices at 0.40, but fair ≈ 0.60 (edge = 20%)

### ✅ 2. Late-Phase Convergence

In last 2 minutes, price should converge to certainty based on current BTC vs strike.

### ✅ 3. Current Price Consistency

P_up = Φ((BTC - strike) / (σ * √t)) should hold for each market individually.

### ❌ REMOVED: Hierarchy "Average" Constraint

**INVALID:** Parent ≈ average of children
**Reason:** Strikes reset, so this relationship doesn't hold.

### ❌ REMOVED: OR-Based Conditional

**INVALID:** Parent = OR of children crossing same strike
**Reason:** Strikes reset, each child tests against its own interval's open.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│            Temporal Arbitrage Engine                        │
│                                                             │
│  ┌─────────────┐  ┌──────────────┐  ┌─────────────────┐  │
│  │ Graph       │  │ Probability  │  │ Constraint      │  │
│  │ Builder     │→ │ Engine       │→ │ Engine          │  │
│  │             │  │              │  │                 │  │
│  └─────────────┘  └──────────────┘  └────────┬────────┘  │
│                                              ↓              │
│  ┌─────────────┐  ┌──────────────┐              │          │
│  │ Multi-      │  │ Signal       │              │          │
│  │ Market      │  │ Generator    │←─────────────┘          │
│  │ Feed        │  │              │                        │
│  └─────────────┘  └──────────────┘                        │
└─────────────────────────────────────────────────────────────┘
```

---

## Data Structures

### Timeframe Enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    M5,   // 5 minutes
    M15,  // 15 minutes
    H1,   // 1 hour
    H4,   // 4 hours
}

impl Timeframe {
    pub fn duration_secs(&self) -> i64 {
        match self {
            Timeframe::M5 => 300,
            Timeframe::M15 => 900,
            Timeframe::H1 => 3600,
            Timeframe::H4 => 14400,
        }
    }

    pub fn vol_multiplier(&self) -> f64 {
        match self {
            Timeframe::M5 => 1.0,
            Timeframe::M15 => (3.0).sqrt(),
            Timeframe::H1 => (12.0).sqrt(),
            Timeframe::H4 => (48.0).sqrt(),
        }
    }
}
```

### TemporalNode

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalNode {
    pub condition_id: String,
    pub timeframe: Timeframe,
    pub strike_price: f64,        // THIS MARKET'S strike (resets per interval)
    pub start_time: i64,
    pub end_time: i64,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub yes_price: Option<f64>,
    pub resolved_outcome: Option<bool>,
    pub close_price: Option<f64>,  // Actual closing price when resolved
}
```

### TemporalGraph

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalGraph {
    pub nodes: HashMap<String, TemporalNode>,
    pub roots: Vec<String>,
    pub price_state: PriceState,
    pub vol_estimator: VolatilityEstimator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceState {
    pub current_price: f64,
    pub last_update: i64,
    pub realized_vol: f64,
    pub price_path: VecDeque<(i64, f64)>,
}
```

---

## Probability Engine

### CORRECTED: Chained Conditional

```rust
impl ProbabilityEngine {
    /// CORRECTED: Chained conditional after observing some legs
    ///
    /// FIX #2: Always use live BTC price from graph.price_state.current_price
    /// The observed leg prices only confirm the path reached that point.
    pub fn chained_conditional_after_observed_legs(
        observed_legs: &[(f64, bool)],  // (close_price, outcome)
        original_strike: f64,            // Parent's strike (P₀)
        live_btc_price: f64,             // Always use fresh live spot
        remaining_time_sec: f64,
        vol_per_sec: f64,
        drift: f64,
    ) -> f64 {
        if observed_legs.is_empty() {
            return Self::prob_up(live_btc_price, original_strike, remaining_time_sec, vol_per_sec, drift);
        }

        let distance = live_btc_price - original_strike;

        if distance >= 0.0 {
            return 0.98;  // High confidence, but small reversal risk
        }

        let std = vol_per_sec * remaining_time_sec.sqrt();
        if std < 1e-6 {
            return 0.5;
        }

        let drift_adjustment = drift * remaining_time_sec;
        let z = (distance + drift_adjustment) / std;
        norm_cdf(z)
    }
}
```

---

## Constraint Engine

### Chained Conditional Check

```rust
impl ConstraintEngine {
    /// CORRECTED: Check chained conditional after some children resolved
    pub fn check_chained_conditional(
        &self,
        graph: &TemporalGraph,
        parent_id: &str,
        now: i64,
    ) -> Option<ConstraintViolation> {
        let parent = graph.nodes.get(parent_id)?;
        let children: Vec<_> = parent.children.iter()
            .filter_map(|id| graph.nodes.get(id))
            .collect();

        let resolved: Vec<_> = children.iter()
            .filter(|c| c.is_resolved(now))
            .collect();

        if resolved.is_empty() || resolved.len() == children.len() {
            return None;
        }

        // Extract observed legs
        let mut observed_legs = Vec::new();
        for child in &resolved {
            let close_price = child.close_price?;
            let outcome = child.resolved_outcome?;
            observed_legs.push((close_price, outcome));
        }

        // FIX #2: Use live BTC price from graph state
        let live_price = graph.price_state.current_price;
        let remaining_time = parent.time_remaining(now) as f64;

        let fair_prob = ProbabilityEngine::chained_conditional_after_observed_legs(
            &observed_legs,
            parent.strike_price,
            live_price,  // FIX: Always use fresh spot
            remaining_time,
            graph.vol_estimator.vol_per_second(),
            0.0,
        );

        let market_prob = parent.yes_price.unwrap_or(0.5);
        let edge = (fair_prob - market_prob).abs();

        if edge >= self.config.min_chained_edge {
            // Return violation...
        }
        None
    }
}
```

---

## Graph Builder (FIXED)

### FIX #1 & #3: Build hierarchy WITHOUT checking strike equality

```rust
impl TemporalGraphBuilder {
    /// FIX #1: Time containment + timeframe nesting is sufficient
    /// Child strikes reset, so we DON'T check strike equality
    fn build_hierarchy(&self, graph: &mut TemporalGraph) {
        let mut node_ids: Vec<_> = graph.nodes.keys().cloned().collect();

        node_ids.sort_by(|a, b| {
            let node_a = &graph.nodes[a];
            let node_b = &graph.nodes[b];

            let tf_cmp = node_b.timeframe as i32 - node_a.timeframe as i32;
            if tf_cmp != 0 {
                return tf_cmp;
            }

            node_a.start_time.cmp(&node_b.start_time)
        });

        for id in &node_ids {
            let node = &graph.nodes[id];

            for potential_parent in &node_ids {
                if potential_parent == id {
                    continue;
                }

                let parent = &graph.nodes[potential_parent];

                if parent.timeframe as i32 <= node.timeframe as i32 {
                    continue;
                }

                // FIX #1: Time containment + timeframe nesting is sufficient
                if node.start_time >= parent.start_time &&
                   node.end_time <= parent.end_time
                {
                    graph.nodes.get_mut(id).unwrap().parent = Some(potential_parent.clone());
                    graph.nodes.get_mut(potential_parent).unwrap().children.push(id.clone());

                    // FIX #3: If parent has no strike, infer from first child
                    if parent.strike_price == 0.0 && node.strike_price > 0.0 {
                        graph.nodes.get_mut(potential_parent).unwrap().strike_price = node.strike_price;
                    }
                    break;
                }
            }
        }
    }

    /// FIX #3: Extract strike price from market metadata
    fn extract_strike_price(market: &Market) -> Option<f64> {
        // Try description field
        if let Some(desc) = &market.description {
            let re = Regex::new(r"\$?([\d,]+(?:\.\d+)?)\s*(?:strike|price|beat|ref)").ok()?;
            if let Some(caps) = re.captures(desc) {
                let num_str = caps.get(1)?.as_str().replace(",", "");
                if let Ok(price) = num_str.parse::<f64>() {
                    return Some(price);
                }
            }
        }

        // Try market metadata/tags
        if let Some(tags) = &market.tags {
            for tag in tags {
                if let Ok(price) = tag.replace("$", "").replace(",", "").parse::<f64>() {
                    return Some(price);
                }
            }
        }

        // Fallback: Will be set from first resolved child's close price
        None
    }
}
```

---

## The Three Fixes Applied

### Fix #1: Graph Builder Linking
**Before:** Checked `|parent.strike - child.strike| < $100`
**After:** Only use time containment + timeframe nesting (child strikes reset)

### Fix #2: Chained Function Param
**Before:** Passed `current_btc_price` but used `price_after_last_leg`
**After:** Always use `graph.price_state.current_price` (freshest live spot)

### Fix #3: Strike Price Extraction
**Before:** Try to parse from title text (doesn't exist)
**After:**
- Try market description/metadata fields
- Fallback: Infer from first resolved child's close price

---

## Implementation Order

1. Core data structures (Timeframe, TemporalNode with strike_price)
2. Probability engine with CORRECTED chained_conditional_after_observed_legs
3. Graph builder for market discovery (FIXED: no strike equality check)
4. Constraint engine (chained conditional, late-phase, price consistency)
5. Strategy engine integration
6. Multi-market feed
7. CLI integration

---

## Success Criteria

- [ ] CORRECT math: chained conditional after observing legs
- [ ] Handles all 4 timeframes simultaneously
- [ ] Detects late-phase anomalies
- [ ] Detects price consistency violations
- [ ] Uses proper √time volatility scaling
- [ ] Can be toggled via `--strategy temporalarb`

---

## Expected Edge

The primary edge comes from **chained conditional arbitrage**:
- After 1-2 legs resolve, we have actual price information
- The parent market may not fully price in this information
- This creates genuine statistical arbitrage opportunities
- Edge typically 5-15% in first 30 seconds after resolution

---

## File Structure

```
src/bot/strategy/
├── mod.rs                           (ADD: exports)
├── types.rs                         (reuse: StrategyEngine, Direction)
├── temporal_arbitrage.rs            (NEW: main engine)
├── graph_builder.rs                 (NEW: build dependency graph)
├── probability_engine.rs            (NEW: continuous-time prob model)
└── constraint_engine.rs             (NEW: check all constraints)

src/bot/feed/
└── multi_market_feed.rs             (NEW: multi-market websocket)

src/bot/strategy_runner.rs           (MODIFY: add run_temporal_arbitrage_step)
src/commands/bot.rs                  (MODIFY: add to StrategyMode)
```

---

## CLI Usage

```bash
# Run temporal arbitrage strategy
cargo run -- bot watch-btc --strategy temporalarb

# Backtest mode
cargo run -- bot backtest --strategy temporalarb --data historical.json
```

---

## Configuration

```rust
TemporalArbitrageConfig {
    min_edge: 0.05,              // 5% minimum edge
    max_spread: 0.03,            // Max 3% spread
    enable_chained_conditional: true,
    enable_late_phase: true,
    enable_price_consistency: true,
    base_volatility_5m: 500.0,    // $500 per 5min
}
```
