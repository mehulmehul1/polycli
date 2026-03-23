# Fair Value Validation Run Analysis

Analysis of the `validate-btc --strategy fairvalue` run (terminal 5, ~687–1051): two BTC 5m markets, one large loss then cooldown/Time blocks.

---

## 1. What Happened (Summary)

| Event | Detail |
|-------|--------|
| **Market 1** | btc-updown-5m-1773522000 (5:00–5:05 PM ET). Strike 70798.22, one YES entry @ 0.38. |
| **Outcome** | BTC dropped (spot 70798 → 70767). YES went 0.38 → 0.09. Position closed −76.3%, bankroll $5 → $4.24. |
| **Market 2** | btc-updown-5m-1773522300 (5:05–5:10 PM). Strike 70766.79. Time-blocked ~10s, then one YES entry @ 0.41. |

So: one big loss in market 1, then one new position in market 2; no duplicate fills (runner correctly ignores Enter when already in a position).

---

## 2. Why BS Fair Value Stays ~0.49

Throughout the run you see:

- `BS=0.494` or `BS=0.493` / `0.487` / `0.488` almost every tick.
- `log_return` between about −0.0005 and 0.0000 (spot very close to strike).

Black–Scholes digital (up/down) in your code is:

- `d2 = -sigma_t/2 + drift`, `prob = normal_cdf(d2)`.
- `t = time_remaining_s / (365.25 * 24 * 3600)` → e.g. 180s ≈ 0.000057 years.
- So `sigma_t = sigma_ann * sqrt(t)` is tiny → `d2` is near 0 → prob near 0.5.

So with **short time to expiry** and **spot ≈ strike**, the model is structurally “at the money”: it gives ~50% no matter what. It does **not** incorporate enough information to say “market is right to price YES at 0.35” vs “YES is cheap at 0.38.” That’s the main structural limit for 5m windows when spot hasn’t moved much.

---

## 3. Why We Bought YES and Lost

- We only enter when **edge_yes = BS_fair − yes_ask ≥ min_edge** (e.g. 0.03).
- With BS_fair ≈ 0.49, that means we buy YES whenever **yes_ask ≤ 0.46** (0.49 − 0.03).
- At first tick we had yes_ask = 0.38 → edge_yes = 0.11 → we entered.
- The market was already pricing YES at 0.38 (and then 0.35, 0.28, …). So the crowd was more bearish on “Up” than our 50% model. In reality BTC drifted down, so the market was right and we were wrong.

So:

- **Logic is consistent**: we only enter when we think fair is above the ask.
- **Model is uninformative**: in this regime BS ≈ 0.5, so we’re effectively “buy when ask &lt; 0.47,” which is not a strong edge when time is short and spot ≈ strike.

---

## 4. Repeated “Decision: ENTER Yes”

The engine outputs **Enter Yes** on many ticks (e.g. 180s, 170s, 160s, …) because:

- It has no notion of “already in a position.”
- Every time `edge_yes ≥ min_edge` it returns `StrategyDecision::Enter`.

The **runner** correctly prevents double entry: `handle_fair_value_entry` returns early when `shadow.is_active()`, so only the first entry is taken. The spam is just logging; behavior is one position per market.

---

## 5. “BLOCKED entry - reason=Time” in Market 2

For the second market you see Time blocks for the first ~10 seconds, then APPROVED at 285s.

- In `risk::trade_allowed`, FairValue uses `min_time_remaining = 30` and also **`contract_age >= 15`**.
- So we block until the contract is at least 15 seconds old.
- At 295s remaining, contract_age is still &lt; 15 → blocked with `FilterReason::Time`.
- Once contract_age ≥ 15 (e.g. 285s remaining), entry is allowed. So the “Time” block here is the **contract_age** rule, not “too much time left.”

---

## 6. Root Causes (Concise)

| Issue | Cause |
|-------|--------|
| **BS ≈ 0.5 always** | Very small `t` and spot ≈ strike → digital option at-the-money. |
| **No predictive edge** | Model doesn’t use order flow / momentum; it only uses spot, strike, vol, time. So when the market is moving (YES 0.38 → 0.09), we don’t adapt. |
| **One-sided risk** | We buy “cheap” YES vs 50%, but if true prob &lt; 50% we lose; with short horizon we’re close to a coin flip. |
| **Vol input** | Realized vol (or default) scales `sigma_t`; with 5m and small moves, vol doesn’t change the “≈0.5” conclusion much. |

---

## 7. Recommendations

1. **Short-horizon / spot ≈ strike**
   - Option A: **Widen min_edge** in this regime (e.g. require edge_yes ≥ 0.05 or 0.07 when `time_remaining &lt; 120` and `|log_return| &lt; 0.001`) so we don’t trade when the model is uninformative.
   - Option B: **No trade when BS is “at the money”** – e.g. skip entry when `0.45 ≤ BS_fair ≤ 0.55` and time_remaining &lt; 2–3 minutes.
   - Option C: Use a **different fair value** for very short tenors (e.g. logit/RN–JD from the paper, or a simple “spot &gt; strike → higher YES” rule with a minimum move threshold).

2. **Single entry per market**
   - Already enforced in the runner. Optionally have the **strategy** return `Hold` when we’re already in a position (would require passing position state into the engine) to reduce log noise and make intent clear.

3. **Contract_age = 15s**
   - Keeping it avoids trading in the first 15s; you can document that “Time” can mean “contract too young” as well as “too little time left.”

4. **Backtest / research**
   - Run the same logic on historical 5m markets: count how often “BS ≈ 0.5, we bought YES at 0.35–0.45” wins vs loses. If it’s near 50%, treat short-dated at-the-money as “no edge” and don’t trade there.

I can implement (1) in the fair-value engine (e.g. “no trade when at-the-money and short time” or higher min_edge in that regime) if you want to proceed that way.
