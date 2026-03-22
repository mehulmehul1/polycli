# Hermes Context: Polycli Trading Bot

## What This Project Is

Polycli is a Rust CLI for Polymarket prediction markets. The bot trades BTC Up/Down 5-minute binary markets using shadow trading (fake trades with real data). No real money is at risk.

## Repository Structure

```
src/
├── main.rs                          # CLI entry point
├── commands/bot.rs                  # All CLI commands (validate-btc, watch-btc, backtest-pmxt, etc.)
├── bot/
│   ├── signal.rs                    # Signal engine (EMA crossover, RSI, Bollinger, entry bands)
│   ├── indicators.rs                # EMA, RSI, Bollinger Bands, momentum slope
│   ├── candles.rs                   # 1m and 5s candle building from ticks
│   ├── strategy/
│   │   ├── types.rs                 # Strategy types (Direction, EntrySignal, etc.)
│   │   ├── heuristic.rs             # Scalper strategy logic
│   │   └── risk.rs                  # Risk gate (position sizing, cooldowns)
│   ├── strategy_runner.rs           # run_shadow_strategy_step() — core trading function
│   ├── shadow.rs                    # Shadow position tracking (bankroll, entry/exit)
│   ├── risk.rs                      # Gatekeeper (spread filter, extreme price, cooldown)
│   ├── pipeline/mod.rs              # Backtest engine, BtStrategy enum, process_fairvalue_snapshot
│   ├── feed_base.rs                 # WebSocket feed, live orderbook processing
│   ├── pricing/                     # FairValue model (logit, Kalman, jump calibrator)
│   ├── recording.rs                 # Tick recorder (saves live data to CSV)
│   └── discovery/                   # Market discovery via Gamma API
```

## Key Commands

```bash
# Build
cargo build --release

# Live shadow trading (records ticks)
cargo run -- bot validate-btc --record --strategy scalper
cargo run -- bot validate-btc --record --strategy late-window
cargo run -- bot validate-btc --record --strategy fair-value

# Backtest on parquet files
cargo run -- bot backtest-pmxt --input-dir pmxtarchives/ --strategy scalper --capital 5

# Backtest on recorded live data
cargo run -- bot backtest-pmxt --input-dir recordings/ --strategy scalper --capital 5

# Export results
cargo run -- bot backtest-pmxt --input-dir recordings/ --strategy scalper --capital 5 --export results.json
```

## Strategies

| Strategy | Flag | Bands | Logic |
|---|---|---|---|
| Scalper | `--strategy scalper` | 0.35-0.65 | EMA crossover + RSI + Bollinger + momentum |
| LateWindow | `--strategy late-window` | 0.85-0.98 | Same engine, tight band filtering |
| FairValue | `--strategy fair-value` | 0.05-0.95 | Logit jump-diffusion + Kalman + jump calibrator |

## Tunable Parameters (in source code)

These are hardcoded. To optimize them, edit these files:

| Parameter | File | Line | Current Value |
|---|---|---|---|
| Entry band low | src/bot/signal.rs | ~44 | 0.35 |
| Entry band high | src/bot/signal.rs | ~45 | 0.65 |
| EMA fast period | src/bot/indicators.rs | ~186 | 3 |
| EMA slow period | src/bot/indicators.rs | ~187 | 6 |
| RSI period | src/bot/indicators.rs | ~280 | 14 |
| BB period | src/bot/indicators.rs | ~349 | 20 |
| BB multiplier | src/bot/indicators.rs | ~350 | 2.0 |
| Momentum threshold | src/bot/signal.rs | ~168 | 0.002 |
| Slope threshold | src/bot/signal.rs | ~168 | -0.002 |
| BB width minimum | src/bot/signal.rs | ~155 | 0.15 |
| Entry window start | src/bot/signal.rs | ~120 | 30s |
| Entry window end | src/bot/signal.rs | ~121 | 280s |
| FairValue min edge | src/bot/pipeline/mod.rs | ~534 | 0.05 |
| Spread filter | src/bot/strategy_runner.rs | ~36 | 0.08 |
| Extreme price | src/bot/strategy_runner.rs | ~47 | 0.92/0.08 |

## Output Format (JSON)

Backtest exports this JSON structure:
```json
{
  "files_processed": 18,
  "markets_processed": 156,
  "trades_taken": 167,
  "wins": 48,
  "losses": 119,
  "win_rate": 28.7,
  "avg_win": 0.3253,
  "avg_loss": -0.1662,
  "profit_factor": 1.96,
  "starting_capital": 5.0,
  "ending_capital": 0.83,
  "total_pnl": -4.17,
  "total_pnl_pct": -83.35,
  "max_drawdown": 87.11,
  "bankroll_history": [["slug", bankroll], ...]
}
```

## Validation Session Output

Each validate-btc run saves to `validation/`:
- `session_YYYYMMDD_HHMMSS.csv` — per-trade data
- `session_YYYYMMDD_HHMMSS.json` — summary with win_rate, net_profit_pct, etc.

## Recordings

Live orderbook ticks saved to `recordings/session_YYYYMMDD_HHMMSS.csv`:
```
timestamp,market_slug,yes_bid,yes_ask,no_bid,no_ask,time_remaining
```

## Workflow for Strategy Optimization

1. Edit strategy parameters in source code
2. `cargo build --release`
3. `target/release/polymarket.exe bot backtest-pmxt --input-dir recordings/ --strategy scalper --capital 5 --export results.json`
4. Read `results.json` — compute fitness score
5. If better: commit. If worse: `git revert HEAD`
6. Repeat

## Fitness Score

```
profit_factor = avg_win / abs(avg_loss)
fitness = (profit_factor * 0.35) + (win_rate/100 * 0.25) + 
          (net_profit_pct/100 * 0.25) + (participation * 0.15)
```

Higher is better. > 0 = profitable. < 0 = losing.

## Git Branches

- `main` — stable code
- Work in separate branches for each strategy optimization
- Merge back to main when fitness improves
