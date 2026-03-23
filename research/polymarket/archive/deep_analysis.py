"""
Deep analysis of gugezh's trading strategy.
Traces every trade through each 5-min market window to find:
- Exact entry timing (seconds into window)
- Entry prices and ladder pattern
- Exit timing and method (SELL vs REDEEM vs loss)
- Per-window P&L
- True win rate
"""
import requests, time, json
from collections import defaultdict
from datetime import datetime, timezone

address = "0xa74330685830ab52bafe35400fae7c9c100725d8"

# Fetch ALL activity
all_events = []
next_cursor = None
for i in range(50):
    url = f"https://data-api.polymarket.com/activity?user={address}&limit=100"
    if next_cursor:
        url += f"&cursor={next_cursor}"
    resp = requests.get(url)
    if resp.status_code != 200: break
    data = resp.json()
    if not data: break
    all_events.extend(data)
    if len(data) < 100: break
    next_cursor = data[-1].get("timestamp")
    time.sleep(0.15)

print(f"Total events: {len(all_events)}")

# Group by market (eventSlug)
markets = defaultdict(lambda: {"buys": [], "sells": [], "redeems": []})

for e in all_events:
    slug = e.get("eventSlug", "unknown")
    t = e.get("type")
    if t == "TRADE":
        side = e.get("side")
        entry = {
            "ts": e.get("timestamp"),
            "price": float(e.get("price", 0)),
            "usdc": float(e.get("usdcSize", 0)),
            "shares": float(e.get("size", 0)),
            "outcome": e.get("outcome"),
            "title": e.get("title", ""),
        }
        if side == "BUY":
            markets[slug]["buys"].append(entry)
        elif side == "SELL":
            markets[slug]["sells"].append(entry)
    elif t == "REDEEM":
        markets[slug]["redeems"].append({
            "ts": e.get("timestamp"),
            "usdc": float(e.get("usdcSize", 0)),
            "title": e.get("title", ""),
        })

# Analyze each market window
results = []
for slug, data in markets.items():
    buys = data["buys"]
    sells = data["sells"]
    redeems = data["redeems"]
    
    if not buys:
        continue
    
    total_buy_usdc = sum(b["usdc"] for b in buys)
    total_buy_shares = sum(b["shares"] for b in buys)
    avg_entry_price = total_buy_usdc / total_buy_shares if total_buy_shares > 0 else 0
    
    total_sell_usdc = sum(s["usdc"] for s in sells)
    total_sell_shares = sum(s["shares"] for s in sells)
    avg_sell_price = total_sell_usdc / total_sell_shares if total_sell_shares > 0 else 0
    
    total_redeem_usdc = sum(r["usdc"] for r in redeems)
    
    total_return = total_sell_usdc + total_redeem_usdc
    pnl = total_return - total_buy_usdc
    pnl_pct = (pnl / total_buy_usdc * 100) if total_buy_usdc > 0 else 0
    
    # Figure out the window start time from slug (e.g., btc-updown-5m-1773447600)
    parts = slug.split("-")
    window_start = 0
    try:
        window_start = int(parts[-1])
    except:
        pass
    
    # Entry timing: how many seconds into the window did first buy happen?
    buy_timestamps = sorted([b["ts"] for b in buys])
    first_buy_delay = buy_timestamps[0] - window_start if window_start > 0 and buy_timestamps else 0
    last_buy_delay = buy_timestamps[-1] - window_start if window_start > 0 and buy_timestamps else 0
    
    # Exit timing
    sell_timestamps = sorted([s["ts"] for s in sells]) if sells else []
    first_sell_delay = sell_timestamps[0] - window_start if window_start > 0 and sell_timestamps else 0
    
    # Determine asset type
    asset_type = "unknown"
    if "btc" in slug: asset_type = "BTC"
    elif "eth" in slug: asset_type = "ETH"
    elif "sol" in slug: asset_type = "SOL"
    elif "xrp" in slug: asset_type = "XRP"
    
    # Determine timeframe
    timeframe = "5m" if "5m" in slug else "15m" if "15m" in slug else "?"
    
    outcome = "WIN" if pnl > 0 else "LOSS" if pnl < -0.5 else "BREAK"
    
    results.append({
        "slug": slug,
        "asset": asset_type,
        "timeframe": timeframe,
        "buy_usdc": total_buy_usdc,
        "buy_shares": total_buy_shares,
        "avg_entry": avg_entry_price,
        "sell_usdc": total_sell_usdc,
        "avg_sell": avg_sell_price,
        "redeem_usdc": total_redeem_usdc,
        "pnl": pnl,
        "pnl_pct": pnl_pct,
        "outcome": outcome,
        "num_buys": len(buys),
        "num_sells": len(sells),
        "first_buy_delay_s": first_buy_delay,
        "last_buy_delay_s": last_buy_delay,
        "first_sell_delay_s": first_sell_delay,
        "entry_direction": buys[0]["outcome"] if buys else "?",
    })

# Sort by PnL
results.sort(key=lambda x: x["pnl"], reverse=True)

# Summary stats
wins = [r for r in results if r["outcome"] == "WIN"]
losses = [r for r in results if r["outcome"] == "LOSS"]
total_markets = len(results)

print(f"\n{'='*80}")
print(f"GUGEZH STRATEGY DEEP ANALYSIS")
print(f"{'='*80}")
print(f"Total distinct markets traded: {total_markets}")
print(f"Wins: {len(wins)} | Losses: {len(losses)} | Win Rate: {len(wins)/total_markets*100:.1f}%")
print(f"Total P&L: ${sum(r['pnl'] for r in results):.2f}")
print(f"Avg win size: ${sum(r['pnl'] for r in wins)/len(wins):.2f}" if wins else "N/A")
print(f"Avg loss size: ${sum(r['pnl'] for r in losses)/len(losses):.2f}" if losses else "N/A")

# Timing analysis
buy_delays = [r["first_buy_delay_s"] for r in results if r["first_buy_delay_s"] > 0]
sell_delays = [r["first_sell_delay_s"] for r in results if r["first_sell_delay_s"] > 0]
print(f"\nTIMING:")
print(f"Avg first buy delay: {sum(buy_delays)/len(buy_delays):.0f}s into window" if buy_delays else "N/A")
print(f"Avg first sell delay: {sum(sell_delays)/len(sell_delays):.0f}s into window" if sell_delays else "N/A")

# Entry price analysis
entry_prices = [r["avg_entry"] for r in results]
print(f"\nENTRY PRICES:")
print(f"Avg entry price: {sum(entry_prices)/len(entry_prices):.3f}")
print(f"Min entry price: {min(entry_prices):.3f}")
print(f"Max entry price: {max(entry_prices):.3f}")

# Exit analysis
sell_prices = [r["avg_sell"] for r in results if r["avg_sell"] > 0]
print(f"\nEXIT PRICES (sells only, excludes redeems):")
if sell_prices:
    print(f"Avg sell price: {sum(sell_prices)/len(sell_prices):.3f}")

# Per-asset breakdown
for asset in ["BTC", "ETH", "SOL", "XRP"]:
    asset_results = [r for r in results if r["asset"] == asset]
    if not asset_results: continue
    asset_wins = [r for r in asset_results if r["outcome"] == "WIN"]
    asset_losses = [r for r in asset_results if r["outcome"] == "LOSS"]
    print(f"\n{asset}: {len(asset_results)} markets | Wins: {len(asset_wins)} | Losses: {len(asset_losses)} | WR: {len(asset_wins)/len(asset_results)*100:.0f}%")

# Show top 10 wins and worst 5 losses
print(f"\n{'='*80}")
print("TOP 10 WINNING TRADES:")
for r in results[:10]:
    print(f"  {r['asset']} {r['timeframe']} | Entry: {r['avg_entry']:.3f} ({r['entry_direction']}) | Sell@{r['avg_sell']:.3f} | ${r['buy_usdc']:.0f} -> ${r['sell_usdc']+r['redeem_usdc']:.0f} | P&L: +${r['pnl']:.2f} ({r['pnl_pct']:.0f}%) | Delay: {r['first_buy_delay_s']}s | Buys:{r['num_buys']} Sells:{r['num_sells']}")

print(f"\nWORST 5 LOSING TRADES:")
for r in results[-5:]:
    print(f"  {r['asset']} {r['timeframe']} | Entry: {r['avg_entry']:.3f} ({r['entry_direction']}) | ${r['buy_usdc']:.0f} -> ${r['sell_usdc']+r['redeem_usdc']:.0f} | P&L: ${r['pnl']:.2f} ({r['pnl_pct']:.0f}%) | Delay: {r['first_buy_delay_s']}s")

# Capital deployment per trade
print(f"\nCAPITAL DEPLOYMENT:")
buy_sizes = [r["buy_usdc"] for r in results]
print(f"Avg capital per market: ${sum(buy_sizes)/len(buy_sizes):.2f}")
print(f"Min capital per market: ${min(buy_sizes):.2f}")
print(f"Max capital per market: ${max(buy_sizes):.2f}")

# How many markets per hour?
all_timestamps = []
for r in results:
    parts = r["slug"].split("-")
    try:
        all_timestamps.append(int(parts[-1]))
    except:
        pass
if all_timestamps:
    all_timestamps.sort()
    span_hours = (all_timestamps[-1] - all_timestamps[0]) / 3600
    print(f"\nFREQUENCY:")
    print(f"Traded over {span_hours:.1f} hours")
    print(f"Markets per hour: {len(results)/span_hours:.1f}")
    print(f"Markets that were NOT played (skipped): in {span_hours:.1f}h, there were {span_hours*12:.0f} possible BTC 5m markets")
    btc_5m = [r for r in results if r["asset"] == "BTC" and r["timeframe"] == "5m"]
    print(f"BTC 5m markets actually played: {len(btc_5m)}")
    print(f"Selectivity: {len(btc_5m)}/{span_hours*12:.0f} = {len(btc_5m)/(span_hours*12)*100:.0f}% of available BTC 5m markets")
