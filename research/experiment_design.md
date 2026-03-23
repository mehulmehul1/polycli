# Polymarket 5-Min BTC Binary Market — Experiment Design v3
# Stages 6-7: CODE_GEN + EXECUTION (Analysis)

## Context

Previous strategies (HawkesFlow, BookValue) produced no edge. Hawkes excitation signals were too noisy on CLOB market-maker-driven books. OBI/VAMP mean reversion failed because Polymarket depth is 2-3 levels — the statistical regularities from deep equity LOBs (Cont et al.) don't transfer.

This document defines 3 new experiments testing fundamentally different hypotheses.

---

## Hypothesis A: "BTC Spot Leading Indicator"

### Premise

Polymarket settles BTC binary markets using Chainlink oracle price feeds. The oracle updates on:
- Every ~10-30 seconds (heartbeat)
- 0.5% price deviation (deviation threshold)

If we can observe BTC spot price changes *faster* than the oracle propagates them on-chain, we can predict the market's resolution direction before the oracle update settles the bet. In the final 60 seconds of a 5-minute window, a sudden 0.3% BTC move will likely resolve the market — but the oracle hasn't updated yet.

### Data Needed

1. **Polymarket market data** (existing pipeline):
   - Market start time, end time, resolution price (YES/NO)
   - Order book snapshots every 1s in final 60s window

2. **BTC spot tick data** (NEW — high-frequency):
   - WebSocket feeds from Binance, Coinbase, Kraken (3 exchanges minimum)
   - Best bid/ask + last trade price, <100ms latency
   - Timestamped with exchange-reported timestamps + local reception time

3. **Chainlink oracle update events** (NEW):
   - All `AnswerUpdated` events from Chainlink BTC/USD feed
   - Ethereum block number + timestamp for each update
   - The deviation between consecutive oracle prices

4. **Polymarket oracle contract events** (NEW):
   - `MarketResolved` events with final price
   - Any oracle update that caused resolution

### Statistical Test

**Primary:** Conditional accuracy of spot-based directional signal in the last 60 seconds.

Define:
- `P_spot(t)` = median BTC spot price across 3 exchanges at time `t`
- `P_oracle(t)` = last Chainlink oracle price at time `t`
- Signal: `direction = sign(P_spot(t) - P_spot(t - 5s))` (5-second momentum)
- Target: `resolution_direction = sign(P_oracle(end) - P_oracle(start))` for the 5-min window

Test: In windows where `|P_spot(t) - P_oracle(t)| > threshold` in the final 60s, does the spot direction predict resolution direction with >55% accuracy?

**Secondary:** Lead-lag analysis between oracle and spot.

```
cross_correlation = corr(P_spot(t + lag), P_oracle(t)) for lag in [-30s, ..., +30s]
```

If oracle lags spot by >5s with correlation >0.95, the edge exists.

### Pseudocode

```python
import websocket
import json
import numpy as np
from collections import deque
from datetime import datetime, timedelta
from scipy import stats

class SpotLeadExperiment:
    def __init__(self):
        self.spot_prices = {
            'binance': deque(maxlen=600),  # 10 min at 1/s
            'coinbase': deque(maxlen=600),
            'kraken': deque(maxlen=600),
        }
        self.oracle_updates = []  # (timestamp, price, block_number)
        self.polymarket_resolutions = []  # (market_id, start, end, resolution)
        self.spot_median = deque(maxlen=600)

    def on_spot_tick(self, exchange, data):
        """Called on each websocket tick from exchanges."""
        price = data['price']
        timestamp = data['timestamp']
        self.spot_prices[exchange].append((timestamp, price))
        self._update_median(timestamp)

    def _update_median(self, timestamp):
        """Compute cross-exchange median price."""
        latest = {}
        for ex, ticks in self.spot_prices.items():
            if ticks:
                latest[ex] = ticks[-1][1]
        if len(latest) >= 2:
            median_price = np.median(list(latest.values()))
            self.spot_median.append((timestamp, median_price))

    def on_oracle_update(self, event):
        """Called on Chainlink AnswerUpdated event."""
        self.oracle_updates.append((
            event['timestamp'],
            event['answer'] / 1e8,  # Chainlink 8 decimals
            event['block_number'],
        ))

    def compute_lead_lag(self, max_lag_seconds=30):
        """Cross-correlation between spot and oracle."""
        if len(self.spot_median) < 100 or len(self.oracle_updates) < 10:
            return None

        # Align timestamps, interpolate oracle to 1s grid
        oracle_interp = self._interpolate_oracle_to_1s()
        spot_series = [p for _, p in self.spot_median]

        min_len = min(len(spot_series), len(oracle_interp))
        spot_series = spot_series[-min_len:]
        oracle_interp = oracle_interp[-min_len:]

        correlations = {}
        for lag in range(-max_lag_seconds, max_lag_seconds + 1):
            if lag > 0:
                corr = np.corrcoef(spot_series[:-lag], oracle_interp[lag:])[0, 1]
            elif lag < 0:
                corr = np.corrcoef(spot_series[-lag:], oracle_interp[:lag])[0, 1]
            else:
                corr = np.corrcoef(spot_series, oracle_interp)[0, 1]
            correlations[lag] = corr

        optimal_lag = max(correlations, key=lambda k: abs(correlations[k]))
        return {
            'optimal_lag_seconds': optimal_lag,
            'max_correlation': correlations[optimal_lag],
            'all_correlations': correlations,
        }

    def test_final_60s_signal(self, markets):
        """
        For each market, in the final 60 seconds:
        - Compute 5s spot momentum
        - Compare to oracle resolution direction
        - Measure accuracy
        """
        results = []
        THRESHOLD_PCT = 0.1  # 0.1% spot-vs-oracle divergence

        for market in markets:
            start = market['start_time']
            end = market['end_time']
            resolution = market['resolution_price']  # 0 or 1

            # Get spot median at end-60s and end-5s
            spot_window = [
                (t, p) for t, p in self.spot_median
                if (end - timedelta(seconds=60)) <= t <= end
            ]
            if len(spot_window) < 10:
                continue

            spot_at_60 = spot_window[0][1]
            spot_at_5 = spot_window[-1][1]
            spot_direction = 1 if spot_at_5 > spot_at_60 else 0

            # Get last oracle price before final 60s
            oracle_before = None
            for t, p, _ in reversed(self.oracle_updates):
                if t <= (end - timedelta(seconds=60)):
                    oracle_before = p
                    break

            if oracle_before is None:
                continue

            # Check divergence: if spot moved significantly from oracle
            divergence = abs(spot_at_5 - oracle_before) / oracle_before

            if divergence > THRESHOLD_PCT:
                correct = (spot_direction == 1 and resolution == 1) or \
                          (spot_direction == 0 and resolution == 0)
                results.append({
                    'market_id': market['id'],
                    'divergence': divergence,
                    'spot_direction': spot_direction,
                    'resolution': resolution,
                    'correct': correct,
                })

        if len(results) < 30:
            return {'n': len(results), 'insufficient_data': True}

        accuracy = sum(r['correct'] for r in results) / len(results)
        # Binomial test: H0 = 0.5, H1 > 0.55
        p_value = stats.binom_test(
            sum(r['correct'] for r in results),
            n=len(results),
            p=0.5,
            alternative='greater',
        )

        return {
            'n': len(results),
            'accuracy': accuracy,
            'p_value': p_value,
            'actionable': accuracy > 0.55 and p_value < 0.05,
            'mean_divergence': np.mean([r['divergence'] for r in results]),
        }

    def _interpolate_oracle_to_1s(self):
        """Linearly interpolate oracle updates to 1-second grid."""
        if len(self.oracle_updates) < 2:
            return []
        times = [t.timestamp() for t, _, _ in self.oracle_updates]
        prices = [p for _, p, _ in self.oracle_updates]
        grid = np.arange(times[0], times[-1], 1.0)
        interpolated = np.interp(grid, times, prices)
        return list(interpolated)
```

### What Result Would Be Actionable

| Result | Interpretation | Action |
|--------|---------------|--------|
| Lag >5s, corr >0.95, accuracy >55%, p<0.05 | Oracle genuinely lags spot. Edge exists in final seconds. | Build real-time spot feed → oracle divergence detector. Trade YES/NO when divergence >0.1% in final 30s. |
| Lag >1s, corr >0.90, accuracy 52-55%, p<0.10 | Weak edge. Oracle lags but not enough to overcome spread. | May be exploitable with tighter execution. Collect more data. |
| Lag <2s, accuracy ~50% | Oracle updates fast enough. No edge. | Abandon. Polymarket's oracle is too fast for this approach. |
| Lag <0 (oracle leads spot) | Polymarket oracle uses a feed that leads public exchanges. | Investigate oracle data source. Possible reverse signal. |

---

## Hypothesis B: "Taker Penalty Fade" (Longshot Bias)

### Premise

Becker (2026) documented that on Kalshi, takers lose ~1.12% systematically due to longshot bias — overpaying for low-probability YES contracts. The mechanism: bettors overweight tail events and underprice the base rate.

On Polymarket 5-min BTC binary markets:
- If BTC is at $95,000 and the market is "BTC > $95,000 in 5 min?", YES should be priced at ~50% (symmetric).
- If BTC is at $94,900 and the strike is $95,000, YES should be <50%.
- Longshot bias means YES is overpriced when it's a longshot, creating a fade-the-YES edge.

### Data Needed

1. **Historical Polymarket 5-min BTC markets** (collect or use existing API):
   - Market ID, strike price, start time, end time
   - YES/NO final resolution
   - Order book snapshots at market open (first 10s)
   - YES midprice at open (or first available price)

2. **BTC spot price at market start** (to compute moneyness):
   - `moneyness = (spot - strike) / spot`
   - `distance_to_strike = |spot - strike|`

3. **Sufficient sample** (minimum 500 markets):
   - Need representation across different moneyness levels
   - Both ITM, ATM, and OTM markets

### Statistical Test

**Primary:** Calibration test — does implied probability (YES price) match realized probability?

Bin markets into deciles by moneyness. For each decile:
- `avg_implied_prob` = average YES midprice
- `realized_prob` = fraction of markets that resolved YES

If calibration is perfect: `realized_prob ≈ avg_implied_prob` for all deciles.
Longshot bias: `realized_prob < avg_implied_prob` for OTM deciles (positive residual).

**Secondary:** Profitability of fade-the-YES strategy.

```
PnL = sum over markets:
    if YES_price > threshold (e.g., 0.55) AND actual_resolution == NO:
        profit = YES_price - 0  (you sold YES, it resolved NO)
    if YES_price > threshold AND actual_resolution == YES:
        loss = 1 - YES_price  (you sold YES, it resolved YES)
```

Test if mean PnL > 0 with t-test against 0.

### Pseudocode

```python
import numpy as np
import pandas as pd
from scipy import stats
from dataclasses import dataclass
from typing import List

@dataclass
class MarketRecord:
    market_id: str
    strike: float
    spot_at_open: float
    yes_price_at_open: float  # midprice, 0-1
    resolution: int  # 1 = YES, 0 = NO
    start_time: float
    end_time: float

class LongshotBiasExperiment:
    def __init__(self, markets: List[MarketRecord]):
        self.df = pd.DataFrame([{
            'market_id': m.market_id,
            'strike': m.strike,
            'spot': m.spot_at_open,
            'yes_price': m.yes_price_at_open,
            'resolution': m.resolution,
            'moneyness': (m.spot_at_open - m.strike) / m.spot_at_open,
            'distance_pct': abs(m.spot_at_open - m.strike) / m.spot_at_open,
        } for m in markets])

    def calibration_test(self, n_bins=10):
        """
        Bin by moneyness. Compare implied vs realized probability.
        Longshot bias: realized < implied for OTM (moneyness < 0).
        """
        df = self.df.copy()
        df['moneyness_bin'] = pd.qcut(df['moneyness'], n_bins, duplicates='drop')

        results = []
        for bin_label, group in df.groupby('moneyness_bin'):
            implied = group['yes_price'].mean()
            realized = group['resolution'].mean()
            residual = realized - implied  # negative = overpriced
            n = len(group)

            # Binomial test: is realized significantly different from implied?
            p_val = stats.binom_test(
                group['resolution'].sum(),
                n=n,
                p=implied,
                alternative='two-sided',
            )

            results.append({
                'moneyness_bin': str(bin_label),
                'n': n,
                'avg_implied': round(implied, 4),
                'avg_realized': round(realized, 4),
                'residual': round(residual, 4),
                'p_value': round(p_val, 4),
                'avg_moneyness': round(group['moneyness'].mean(), 4),
            })

        return pd.DataFrame(results)

    def fade_yes_strategy(self, min_yes_price=0.55):
        """
        Fade (sell) YES when price > min_yes_price.
        Simulate PnL per contract.
        """
        candidates = self.df[self.df['yes_price'] >= min_yes_price].copy()

        if len(candidates) < 30:
            return {'n': len(candidates), 'insufficient_data': True}

        # PnL per contract: if sold YES at price P
        # Resolution YES: lose (1 - P)
        # Resolution NO: gain P
        candidates['pnl'] = np.where(
            candidates['resolution'] == 0,
            candidates['yes_price'],          # profit = sold YES price
            -(1 - candidates['yes_price']),   # loss = 1 - sold price
        )

        mean_pnl = candidates['pnl'].mean()
        std_pnl = candidates['pnl'].std()
        n = len(candidates)
        t_stat, p_value = stats.ttest_1samp(candidates['pnl'], 0, alternative='greater')

        # ROI = mean_pnl / capital_at_risk (capital at risk = 1 - yes_price for YES sales)
        avg_capital_risk = (1 - candidates['yes_price']).mean()
        roi = mean_pnl / avg_capital_risk if avg_capital_risk > 0 else 0

        return {
            'n': n,
            'mean_pnl_per_contract': round(mean_pnl, 4),
            'std_pnl': round(std_pnl, 4),
            'roi_pct': round(roi * 100, 2),
            't_stat': round(t_stat, 3),
            'p_value': round(p_value, 4),
            'actionable': mean_pnl > 0 and p_value < 0.05 and roi > 0.02,
            'win_rate': round(candidates['resolution'].value_counts(normalize=True).get(0, 0), 4),
        }

    def fade_by_moneyness(self):
        """
        Test if fade edge is concentrated in specific moneyness buckets.
        Most actionable: find the bucket with strongest signal.
        """
        df = self.df.copy()
        df['moneyness_bucket'] = pd.cut(
            df['moneyness'],
            bins=[-np.inf, -0.005, -0.002, 0, 0.002, 0.005, np.inf],
            labels=['deep_otm', 'otm', 'slightly_otm', 'atm', 'itm', 'deep_itm'],
        )

        results = []
        for bucket, group in df.groupby('moneyness_bucket', observed=True):
            if len(group) < 10:
                continue
            implied = group['yes_price'].mean()
            realized = group['resolution'].mean()

            # Fade YES strategy in this bucket
            fade_pnl = np.where(
                group['resolution'] == 0,
                group['yes_price'],
                -(1 - group['yes_price']),
            )
            t_stat, p_val = stats.ttest_1samp(fade_pnl, 0, alternative='greater')

            results.append({
                'bucket': bucket,
                'n': len(group),
                'avg_yes_price': round(implied, 4),
                'realized_yes_rate': round(realized, 4),
                'residual': round(realized - implied, 4),
                'fade_mean_pnl': round(fade_pnl.mean(), 4),
                'fade_p_value': round(p_val, 4),
            })

        return pd.DataFrame(results)
```

### What Result Would Be Actionable

| Result | Interpretation | Action |
|--------|---------------|--------|
| OTM residual < -2%, p<0.05, ROI >2% | Clear longshot bias. YES systematically overpriced when BTC is far from strike. | Sell YES when spot is >0.3% from strike. Size by Kelly criterion on empirical edge. |
| Calibration flat, no bucket shows residual | No longshot bias on Polymarket. | Abandon. Markets are well-calibrated (surprising but possible). |
| Bias exists but only for deep OTM (rare) | Edge exists but low frequency. | Not enough trades to overcome variance. Need longer collection period. |
| Reverse bias (YES underpriced) | Possible favorite bias or market-maker subsidy. | Investigate reverse strategy (buy YES when ATM). |

---

## Hypothesis C: "Time-of-Day Pattern"

### Premise

BTC volatility and directional movement have well-documented intraday patterns:
- **Asian session (00:00-08:00 UTC):** Lower volume, trend continuation
- **European session (08:00-14:00 UTC):** Moderate volatility
- **US session (14:00-21:00 UTC):** Highest volume, mean-reverting
- **US close / Asian open (21:00-00:00 UTC):** Volatility spike

If BTC has a higher probability of moving UP during certain hours, a binary market starting in that hour has a biased base rate. The naive 50% YES price doesn't account for this.

### Data Needed

1. **Historical Polymarket 5-min BTC market outcomes** (or proxy via BTC price):
   - Start time (UTC hour + minute)
   - Resolution: did BTC go UP or DOWN in that 5-min window?
   - 6+ months of data for statistical power

2. **BTC 5-min OHLCV candles** (fallback if Polymarket data unavailable):
   - Binance API: `GET /api/v3/klines` with `interval=5m`
   - Close price of candle N vs candle N-1 → direction
   - 1 year of data = ~105,000 5-min candles

3. **Session labels:**
   - Map UTC hour → session bucket

### Statistical Test

**Primary:** Chi-squared test of independence between hour-of-day and BTC direction.

```
H0: P(UP | hour) = P(UP) for all hours (no hour effect)
H1: At least one hour has P(UP) ≠ P(UP) (hour effect exists)
```

Construct 24x2 contingency table (hour × UP/DOWN). Run chi-squared test.

**Secondary:** For each hour, compute:
- `edge = P(UP | hour) - 0.5`
- Test if `|edge| > 0.03` with binomial test
- Compute expected value of always-betting-YES in that hour

### Pseudocode

```python
import numpy as np
import pandas as pd
from scipy import stats
from collections import defaultdict

class TimeOfDayExperiment:
    def __init__(self):
        self.candles = None  # DataFrame with columns: timestamp, open, high, low, close, volume
        self.hourly_stats = defaultdict(lambda: {'up': 0, 'down': 0, 'flat': 0})

    def load_binance_5m_candles(self, symbol='BTCUSDT', months=12):
        """
        Fetch historical 5-min candles from Binance.
        Returns DataFrame with columns: timestamp, open, high, low, close, volume
        """
        import requests

        all_candles = []
        end_time = int(pd.Timestamp.now().timestamp() * 1000)
        start_time = end_time - (months * 30 * 24 * 60 * 60 * 1000)

        while start_time < end_time:
            resp = requests.get('https://api.binance.com/api/v3/klines', params={
                'symbol': symbol,
                'interval': '5m',
                'startTime': start_time,
                'endTime': end_time,
                'limit': 1000,
            })
            data = resp.json()
            if not data:
                break
            all_candles.extend(data)
            start_time = data[-1][0] + 1  # next ms after last candle

        self.candles = pd.DataFrame(all_candles, columns=[
            'open_time', 'open', 'high', 'low', 'close', 'volume',
            'close_time', 'quote_volume', 'trades', 'taker_buy_base',
            'taker_buy_quote', 'ignore',
        ])
        self.candles['open_time'] = pd.to_datetime(self.candles['open_time'], unit='ms')
        self.candles['close'] = self.candles['close'].astype(float)
        self.candles['open'] = self.candles['open'].astype(float)
        self.candles['hour_utc'] = self.candles['open_time'].dt.hour

        return self.candles

    def compute_direction(self):
        """Compute UP/DOWN for each 5-min candle vs previous candle close."""
        df = self.candles.copy()
        df['prev_close'] = df['close'].shift(1)
        df['direction'] = np.where(
            df['close'] > df['prev_close'], 'UP',
            np.where(df['close'] < df['prev_close'], 'DOWN', 'FLAT'),
        )
        df = df.dropna(subset=['prev_close'])
        self.candles = df
        return df

    def hourly_contingency_table(self):
        """Build 24x2 table of hour × direction."""
        df = self.compute_direction()

        # Exclude FLAT (rare, ~0.1% of candles)
        df = df[df['direction'] != 'FLAT']

        table = pd.crosstab(df['hour_utc'], df['direction'])
        # Ensure both UP and DOWN columns exist
        if 'UP' not in table.columns:
            table['UP'] = 0
        if 'DOWN' not in table.columns:
            table['DOWN'] = 0

        return table[['UP', 'DOWN']]

    def chi_squared_test(self):
        """Test independence of hour and direction."""
        table = self.hourly_contingency_table()
        chi2, p_value, dof, expected = stats.chi2_contingency(table)

        return {
            'chi2_statistic': round(chi2, 3),
            'p_value': round(p_value, 6),
            'degrees_of_freedom': dof,
            'significant': p_value < 0.05,
            'contingency_table': table,
            'expected_frequencies': pd.DataFrame(
                expected, index=table.index, columns=table.columns,
            ),
        }

    def hourly_edge(self):
        """Compute per-hour edge and significance."""
        table = self.hourly_contingency_table()
        total_per_hour = table['UP'] + table['DOWN']
        up_rate = table['UP'] / total_per_hour

        results = []
        for hour in range(24):
            if hour not in table.index:
                continue
            n = total_per_hour[hour]
            up_count = table.loc[hour, 'UP'] if hour in table.index else 0
            rate = up_rate[hour] if hour in up_rate.index else 0.5
            edge = rate - 0.5

            # Binomial test: is P(UP) significantly different from 0.5?
            p_val = stats.binom_test(up_count, n=n, p=0.5, alternative='two-sided')

            # Session label
            if 0 <= hour < 8:
                session = 'Asian'
            elif 8 <= hour < 14:
                session = 'European'
            elif 14 <= hour < 21:
                session = 'US'
            else:
                session = 'US_Close'

            results.append({
                'hour_utc': hour,
                'session': session,
                'n': n,
                'up_count': up_count,
                'up_rate': round(rate, 4),
                'edge': round(edge, 4),
                'p_value': round(p_val, 6),
                'significant': p_val < 0.05 and abs(edge) > 0.02,
            })

        return pd.DataFrame(results)

    def profitability_simulation(self, min_edge=0.02, bet_size=1.0):
        """
        Simulate betting YES in hours with edge > min_edge,
        betting NO in hours with edge < -min_edge.
        """
        edge_df = self.hourly_edge()
        actionable_hours = edge_df[abs(edge_df['edge']) >= min_edge].copy()

        if len(actionable_hours) == 0:
            return {'actionable_hours': 0, 'no_edge': True}

        # Simulate: for each candle in actionable hours, bet on predicted direction
        candles = self.compute_direction()
        candles = candles[candles['direction'] != 'FLAT']

        total_pnl = 0
        total_bets = 0

        for _, row in actionable_hours.iterrows():
            hour = row['hour_utc']
            edge = row['edge']
            hour_candles = candles[candles['hour_utc'] == hour]

            if edge > 0:
                # Bet YES (UP)
                wins = (hour_candles['direction'] == 'UP').sum()
            else:
                # Bet NO (DOWN)
                wins = (hour_candles['direction'] == 'DOWN').sum()

            bets = len(hour_candles)
            pnl = (wins * bet_size) - (bets * bet_size * 0.5)  # simplified: assume 50% implied
            total_pnl += pnl
            total_bets += bets

        roi = total_pnl / (total_bets * bet_size) if total_bets > 0 else 0

        return {
            'actionable_hours': len(actionable_hours),
            'total_bets': total_bets,
            'simulated_roi_pct': round(roi * 100, 2),
            'actionable_hour_details': actionable_hours.to_dict('records'),
            'actionable': roi > 0.01,
        }

    def session_summary(self):
        """Aggregate edge by trading session."""
        edge_df = self.hourly_edge()
        session_stats = edge_df.groupby('session').agg({
            'n': 'sum',
            'up_count': 'sum',
            'edge': ['mean', 'std'],
        }).round(4)

        return session_stats
```

### What Result Would Be Actionable

| Result | Interpretation | Action |
|--------|---------------|--------|
| Chi-squared p<0.01, multiple hours with |edge|>2%, p<0.05 | Strong intraday pattern. Certain hours systematically favor UP or DOWN. | Only trade markets starting in high-edge hours. Bet in predicted direction. |
| Chi-squared p<0.05, 1-2 hours with edge but high variance | Weak pattern. Possible overfitting. | Collect more data. Test on out-of-sample period. |
| Chi-squared p>0.05, no hour shows significant edge | No intraday pattern at 5-min granularity. | Abandon time-of-day. Try longer windows (15-min, 1-hour markets). |
| Session-level edge but not hourly | Pattern exists at session level, not hour level. | Trade based on session (e.g., "always bet NO during US session"). |

---

## Experiment Priority & Timeline

| Priority | Hypothesis | Data Complexity | Expected Runtime | Expected Edge |
|----------|-----------|----------------|-----------------|---------------|
| 1 | A: Spot Leading | HIGH (3 websocket feeds + oracle events) | 2-3 days to build feed, 1 week to collect | HIGH if oracle lags >5s |
| 2 | C: Time of Day | LOW (Binance API, public) | 2 hours to collect, 1 day to analyze | MODERATE (well-known pattern) |
| 3 | B: Longshot Bias | MEDIUM (need Polymarket historical data) | 1 day to collect, 1 day to analyze | MODERATE (Becker found 1.12% on Kalshi) |

### Recommended Execution Order

1. **Start with Hypothesis C** (lowest data cost, fastest to validate/refute). Run `load_binance_5m_candles` + `chi_squared_test` first. If no hourly pattern exists, save the effort.

2. **Then Hypothesis B** (requires Polymarket data but no infrastructure). Build the historical dataset of 5-min market outcomes + prices. Run calibration test.

3. **Then Hypothesis A** (highest potential edge but most infrastructure). Only worth building if A or B show promise, or as a longer-term research project.

### Failure Criteria (for all experiments)

- **Minimum sample:** n ≥ 200 observations per test
- **Significance:** p < 0.05 after Bonferroni correction for multiple comparisons
- **Effect size:** Edge > 1% (after Polymarket's 2% fee on winnings)
- **Out-of-sample:** If tuned on period 1, must validate on period 2

---

## Appendix: Data Sources

| Source | API/Method | Rate Limit | Cost |
|--------|-----------|------------|------|
| Binance spot | REST + WebSocket | 1200 req/min | Free |
| Coinbase spot | WebSocket | 100 req/s | Free |
| Kraken spot | WebSocket | 1 req/s (REST) | Free |
| Chainlink oracle | Ethereum RPC (Infura/Alchemy) | Plan-dependent | Free tier available |
| Polymarket markets | CLOB API + subgraph | Varies | Free |
| Polymarket historical | Gamma API | Varies | Free |

## Appendix: Risk Considerations

1. **Fee impact:** Polymarket charges 2% on net winnings. Any edge must exceed 2% ROI to be profitable.
2. **Liquidity:** 5-min BTC markets have thin books. Large positions will move the price against you.
3. **Oracle behavior:** Chainlink may update faster than expected. Hypothesis A may be invalidated by sub-second oracle updates.
4. **Regime changes:** Intraday patterns (Hypothesis C) may shift over time. Need rolling validation.
5. **Data snooping:** With 3 hypotheses, apply Bonferroni correction: require p < 0.05/3 ≈ 0.0167 for significance.
