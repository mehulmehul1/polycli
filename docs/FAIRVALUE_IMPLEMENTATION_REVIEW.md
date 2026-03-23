# Fair Value Strategy — Implementation Review

This document reviews the current implementation against the intended design (two-engine system: Black–Scholes “source of truth” + Logit Jump–Diffusion “market filter”) and the [arxiv:2510.15205](https://arxiv.org/html/2510.15205v1) paper.

---

## 1. What the Design Claims

| Component | Intended behavior |
|-----------|-------------------|
| **Engine A (Source of Truth)** | Actual BTC/USD from Polymarket RTDS (Chainlink). Black–Scholes computes fair probability from spot vs strike (BTC at open). |
| **Engine B (Market Filter)** | YES/NO prices → RN–JD (Logit Jump–Diffusion) filters noise → “Filtered Market Belief” (e.g. 58%). |
| **Edge** | `edge = BS_fair − market_fair`. Enter YES when edge > min_edge, NO when edge < −min_edge. |
| **Spot / Strike** | Spot = real BTC/USD; Strike = BTC at market open. No circular use of market token price. |

---

## 2. What Is Implemented

### 2.1 Engine A (Spot + Black–Scholes)

- **Spot source**
  - `FairValueEngine::with_rtds_feed("btc/usd")` is used in `watch_btc_market` when strategy is FairValue (`commands/bot.rs`).
  - Composite feed: primary = RTDS (WebSocket), secondary = `DerivedSpotFeed` (never updated in the watch loop).
  - `get_spot_price()`: if `use_external_spot` and composite has a price, use it; else **fallback = `obs.yes_mid`** (market token mid).
  - **Strike (`spot_at_open`)**: set only when `feed.is_healthy()` and `feed.get_price().is_some()`. Otherwise stays `None` → **`unwrap_or(1.0)`** in `decide()`.

- **Problems**
  1. **Circular logic when RTDS is missing or slow**
     - If RTDS has not yet sent a message (or connection fails), composite returns no price → spot = `obs.yes_mid`, strike = `1.0` (default). So we still trade using market mid as “spot” and a constant strike, which is not the intended design.
  2. **Strike default `1.0`**
     - With strike = 1.0 and spot = 0.9, BS digital gives a low fair YES prob → large negative “edge” vs logit (e.g. 0.9) → Enter NO. So we can enter on a bogus edge derived from wrong inputs.
  3. **RTDS “healthy” before first price**
     - `PolymarketRtdsFeed::is_healthy()` uses `last_update`; before any message `last_update == 0`, so for the first 30s we have `now - 0 < 30` → healthy, but `get_price() == None`. So we do not set `spot_at_open` from RTDS until the first message; that part is correct. But we still use `obs.yes_mid` for spot when no price is available, which reintroduces circularity.
  4. **Secondary never updated**
     - `CompositeSpotFeed::update_derived(yes_mid, no_mid)` exists but is never called from the watch loop or from `FairValueEngine`. So the secondary feed never gets a price; when primary has no data, we rely on `obs.yes_mid` in `get_spot_price()`, not on a “derived” feed.

- **Black–Scholes**
  - `FairValueModel::fair_prob_updown(spot, spot_at_open, time_remaining, realized_vol)` is a standard digital-option style formula (log return, d2, normal CDF). Correct for the intended inputs (real spot/strike in USD).

### 2.2 Engine B (Logit “Market Filter”)

- **Implementation**
  - `LogitJumpDiffusion` in `pricing/logit_model.rs`: maintains `current_logit` / `current_prob` and belief vol.
  - `update(obs)`: **exponential smoothing** of observed logit with adaptive gain from spread/volume. No EM, no RN drift, no jump separation.
  - `FilteredState.prob` is used as “market fair prob” in the edge formula.

- **Gap vs paper**
  - The paper uses a full **RN logit jump–diffusion**: state filter (e.g. Kalman) with **microstructure noise**, **EM for diffusion vs jumps**, and **risk-neutral drift**. The current code is a simple exponential smoother in logit space, not the calibrated RN–JD kernel. So “Engine B” is a lightweight filter, not the full arxiv pipeline.

### 2.3 Edge and Entry Logic

- **Formula**
  - `edge = bs_fair_prob - market_fair_prob` (BS minus logit-filtered prob). Positive → YES, negative → NO. Implemented as in the design.
- **Entry condition**
  - `if edge.abs() < min_edge { Hold }` then `direction = if edge > 0 { Yes } else { No }`. So we **do** enter when edge is negative (we take NO). That is consistent with “bet against the market when BS disagrees.”
- **Execution vs edge**
  - Edge is **not** checked against the price we pay. We do not require e.g. `bs_fair - yes_ask >= min_edge` for YES or `(1 - bs_fair) - no_ask >= min_edge` for NO. So we can enter YES when the ask is above our fair (overpaying) or NO when the no_ask is above our fair NO value. The design doc describes edge as “Math 62% vs Market 58%” but does not explicitly tie entry to ask; adding an ask check would make execution consistent with value.

### 2.4 CalibratedFairValue

- **Role**
  - Wraps `FairValueModel`, adds spread adjustment, book imbalance, historical bias. Produces `fair_prob_calibrated` and edges vs yes_ask/no_ask internally, but the **strategy** uses only `fair_prob_calibrated` and compares it to the **logit** market prob, not to the ask.
- **Debug log**
  - `[FAIRVALUE DEBUG] spot_at_open=… spot=…` in `calibrated.rs` confirms that when both spot and strike are 0.9 (or similar), we are effectively using the same (market-derived) number for both, i.e. circular inputs to BS.

---

## 3. Root Cause of “Not Working”

From the run you observed:

- Log showed `spot_at_open=0.9000 spot=0.9000` and `edge=-0.4222`, then Enter NO and a loss.
- That implies either:
  1. RTDS had not yet sent a real BTC/USD price, and we fell back to `obs.yes_mid` for spot while strike was set from the same or another fallback (e.g. composite returning a 0–1 value from somewhere), or  
  2. RTDS sent a value that was interpreted as 0.9 (e.g. wrong payload or normalized price).

In both cases, **BS is being fed non-USD or circular inputs**, so “fair value” and “edge” are not meaningful. The fix is to **never trade until we have a real BTC/USD spot and strike from the external feed**, and to avoid using `obs.yes_mid` (or any 0–1 market mid) as spot or strike.

---

## 4. Recommendations

### 4.1 Must-fix (correctness)

1. **No trading without real spot and strike**
   - In `decide()`, if `spot_at_open.is_none()` **or** if `get_spot_price(obs)` is coming from fallback (e.g. you can add a `spot_source()` or “from_external” flag on the feed), return `StrategyDecision::Hold` and do not enter.
   - Remove or tighten `spot_at_open.unwrap_or(1.0)` so we never use a default strike when the design requires “BTC at open.”

2. **Spot must be BTC/USD**
   - When using composite, only treat the feed as valid if the active source is RTDS (or Chainlink) and the value is in a plausible USD range (e.g. 10_000 < price < 500_000 for BTC). If the feed returns a value in [0, 1], treat it as invalid and do not use it for spot or strike.

3. **Optional but recommended: edge vs ask**
   - Before `StrategyDecision::Enter`, require:
     - Enter YES only if `bs_fair_prob - yes_ask >= min_edge`;
     - Enter NO only if `(1.0 - bs_fair_prob) - no_ask >= min_edge`.
   This avoids entering when the quoted ask is above our fair value.

### 4.2 Align with paper (later)

4. **Full RN–JD for Engine B**
   - Replace the exponential smoother with: Kalman (or similar) in logit space with heteroskedastic measurement noise, then EM on increments for σ_b, λ, jump moments, and RN drift re-smoothing. Use the resulting filtered state as “market belief” and optionally for variance forecasts.

5. **Do not use market mid as strike**
   - Document and enforce: strike is set **once** from the external feed when the market is considered “open” (e.g. first healthy RTDS price after discovery), and never from `obs.yes_mid` or derived feed fed by the same market.

---

## 5. Summary Table

| Claim in design | Current implementation | Status |
|-----------------|------------------------|--------|
| Spot = actual BTC/USD (Chainlink/RTDS) | RTDS wired; fallback = yes_mid when no price | ⚠️ Circular when RTDS absent/slow |
| Strike = BTC at open | Set only from feed; default 1.0 when None | ⚠️ Default wrong; can be set from bad feed |
| Market view = Logit-filtered belief | Exponential smoother in logit (no EM/RN) | ⚠️ Simplified vs paper |
| Edge = BS_fair − market_fair | Implemented as such | ✅ |
| Enter only when \|edge\| ≥ min_edge | Implemented; direction from sign(edge) | ✅ |
| No trading on circular spot/strike | Not enforced | ❌ |

**Bottom line:** The two-engine structure and edge formula are in place, but when the external feed does not provide real BTC/USD (or is slow), the code falls back to market mid and/or default strike, which recreates the circular logic the design was meant to remove. Enforcing “trade only when we have real spot and strike from RTDS” and validating that spot is in USD range will align behavior with the design and prevent the kind of loss you observed.
