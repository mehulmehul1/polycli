# 🔍 Polymarket Research Hub: Methodology

This document outlines the technical framework for identifying and replicating high-win-rate bots.

## 1. Market-First Discovery (`market_scanner.py`)

Our primary discovery engine uses a **market-first approach** instead of random proxy contract scanning.

### How It Works:
1. **Slug Generation:** Generate market slugs for the last N hours
   - Pattern: `{crypto}-updown-{interval}-{unix_timestamp}`
   - Example: `btc-updown-5m-1773463800`
   - 5m markets align to 300-second intervals
   - 15m markets align to 900-second intervals

2. **Trader Discovery:** For each market slug:
   - Fetch market data from **Gamma API** to get `conditionId`
   - Query **Data API** `/holders` endpoint with `conditionId`
   - Extract all `proxyWallet` addresses

3. **Candidate Filtering:** Analyze each trader against bot criteria:
   - **Win Rate:** ≥ 52% (configurable)
   - **Avg Bet Size:** ≤ $100 (to filter whales)
   - **Participation Rate:** ≥ 10% (automated trading indicator)
   - **Minimum Markets:** ≥ 20 trades in target markets

### API Endpoints:
```bash
# Step 1: Get conditionId from slug
GET https://gamma-api.polymarket.com/events?slug=btc-updown-5m-1773463800

# Step 2: Get holders from conditionId
GET https://data-api.polymarket.com/holders?market=0x{conditionId}&limit=20

# Step 3: Get trader activity for analysis
GET https://data-api.polymarket.com/activity?user={address}&limit=500
```

### Participation Rate Formula:
```
participation_rate = (actual_windows / possible_windows) * 100
```
Where `possible_windows = (time_span / 300) + 1` for 5-minute markets.

---

## 2. Legacy Proxy Scanning (`bot_scanner.py`)
**Deprecated** - Previously scraped the Polymarket Proxy contract (`0x4d9...`) via Etherscan V2.
- Randomly sampled recent transactions
- Less efficient than market-first approach
- Still available for cross-reference

---

## 3. Bankroll & PnL Auditing (`pnl_verifier.py`)
We use a **Dual-Track Verification** system to cross-reference claims.
- **Track 1: On-Chain USDC Transfers (Ground Truth Bankroll):**
  - Downloads every USDC.e ERC-20 transfer event via Polygonscan.
  - This confirms the **Start Amount** (e.g., $5 to $50) and **Organic Growth**. If a user deposits $10k halfway, we'll see it here.
- **Track 2: Polymarket API Activity (Performance Statistics):**
  - Downloads the complete trade/redeem log from Polymarket's data servers.
  - This verifies the **Win Rate** and **Execution Timing** (the ~55,000 events mentioned earlier).

## 4. Signal & Timing Spectrum (`signal_analyzer.py`)
The "Capture Delay" (seconds since window start) allows us to reverse-engineer a bot's specific alpha source.

### The Strategy Spectrum:
| Delay Window | Phase | Strategy Profile |
| :--- | :--- | :--- |
| **0s - 30s** | Early | **Open Gap / News Arb:** Bets on the initial gap between the strike price and live spot price. |
| **30s - 240s** | Mid | **Mean Reversion / RSI:** Bets on price returning to the mean after an initial move. |
| **240s - 295s** | Late | **Momentum / Breakout:** Bets on a strong trend continuing into the expiry (e.g., `0xa743`). |
| **295s - 300s** | Sniping | **Last-Second Scalp:** High-frequency attempts to be the very last trade before the window closes. |
| **> 301s** | Post-Expiry | **Latency Arbitrage:** Picking off stale limit orders after the price is locked (e.g., `0x9ce0`). |

## 5. Target Bot Profile

We target **small-bankroll automated bots** with proven edge in crypto UpDown markets:

| Criteria | Threshold | Rationale |
|----------|-----------|-----------|
| **Win Rate** | ≥ 52% | Above random, sustainable edge |
| **Avg Bet** | ≤ $100 | Filters institutional whales |
| **Participation** | ≥ 10% | Indicates automated/systematic trading |
| **Markets** | ≥ 20 | Sufficient sample size |
| **Specialization** | Crypto 5m/15m | Focus on UpDown binary markets |

### Why This Profile?
- **Replicable:** Small bankrolls ($5-$250 starting capital) are accessible to retail
- **Scalable:** 5m/15m markets run 288/96 times daily = 384 opportunities/day
- **API-Accessible:** All data available via public Polymarket APIs

---

## 6. Wallet Profiling (`wallet_profiler.py`)
Determines the "Trading Persona":
- **The Compounder:** Steady sizing growth, high win rate, directional.
- **The Arb-Wall:** Massive volume, low individual profit, bets both sides to capture spreads.
- **The Sniper:** Low frequency, high accuracy, enters only in the last 5 seconds.
- **The High-Frequency Bot:** >90% participation, trades nearly every window.

---

## 7. Usage Examples

```bash
# Basic 24-hour scan (all cryptos, 5m + 15m)
python research/polymarket/discovery/market_scanner.py --hours 24

# Target specific cryptos and intervals
python research/polymarket/discovery/market_scanner.py --hours 12 --cryptos btc eth --intervals 5m

# Adjust filters for stricter criteria
python research/polymarket/discovery/market_scanner.py --hours 24 --min-wr 55 --max-avg-bet 50 --min-part 20

# Save results to JSON
python research/polymarket/discovery/market_scanner.py --hours 24 --output candidates.json
```

---

## 8. Research Output Format

Each candidate includes:
```json
{
  "address": "0x...",
  "total_markets": 269,
  "win_rate": 85.1,
  "wins": 180,
  "losses": 31,
  "avg_bet": 82.95,
  "total_in": 22313.45,
  "total_out": 22684.98,
  "net_pnl": 371.53,
  "participation_rate": 11.1,
  "first_trade": "2025-03-13T...",
  "last_trade": "2025-03-14T..."
}
```
