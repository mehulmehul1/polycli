"""
Deep analysis of gugezh - output key metrics only
"""
import requests, time
from collections import defaultdict

address = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"

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

# Group by market
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
        }
        if side == "BUY": markets[slug]["buys"].append(entry)
        elif side == "SELL": markets[slug]["sells"].append(entry)
    elif t == "REDEEM":
        markets[slug]["redeems"].append({"usdc": float(e.get("usdcSize", 0))})

results = []
for slug, data in markets.items():
    buys = data["buys"]
    sells = data["sells"]
    redeems = data["redeems"]
    if not buys: continue
    
    total_buy_usdc = sum(b["usdc"] for b in buys)
    total_buy_shares = sum(b["shares"] for b in buys)
    avg_entry = total_buy_usdc / total_buy_shares if total_buy_shares > 0 else 0
    total_sell_usdc = sum(s["usdc"] for s in sells)
    total_redeem_usdc = sum(r["usdc"] for r in redeems)
    pnl = (total_sell_usdc + total_redeem_usdc) - total_buy_usdc
    
    parts = slug.split("-")
    window_start = 0
    try: window_start = int(parts[-1])
    except: pass
    
    buy_ts = sorted([b["ts"] for b in buys])
    first_buy_delay = buy_ts[0] - window_start if window_start > 0 else 0
    
    asset = "BTC" if "btc" in slug else "ETH" if "eth" in slug else "SOL" if "sol" in slug else "XRP" if "xrp" in slug else "?"
    tf = "5m" if "5m" in slug else "15m" if "15m" in slug else "?"
    outcome = "WIN" if pnl > 0.5 else "LOSS" if pnl < -0.5 else "BREAK"
    
    results.append({
        "slug": slug, "asset": asset, "tf": tf,
        "buy_usdc": total_buy_usdc, "avg_entry": avg_entry,
        "sell_usdc": total_sell_usdc, "redeem_usdc": total_redeem_usdc,
        "pnl": pnl, "outcome": outcome,
        "num_buys": len(buys), "num_sells": len(sells),
        "first_delay": first_buy_delay,
        "direction": buys[0]["outcome"] if buys else "?",
    })

wins = [r for r in results if r["outcome"] == "WIN"]
losses = [r for r in results if r["outcome"] == "LOSS"]
breaks = [r for r in results if r["outcome"] == "BREAK"]

print("=== GUGEZH STRATEGY DEEP ANALYSIS ===")
print(f"Total markets traded: {len(results)}")
print(f"WINS: {len(wins)} | LOSSES: {len(losses)} | BREAK-EVEN: {len(breaks)}")
print(f"WIN RATE: {len(wins)/len(results)*100:.1f}%")
print(f"Total PnL: ${sum(r['pnl'] for r in results):.2f}")
if wins:
    print(f"Avg WIN: +${sum(r['pnl'] for r in wins)/len(wins):.2f}")
if losses:
    print(f"Avg LOSS: ${sum(r['pnl'] for r in losses)/len(losses):.2f}")

# Timing
delays = [r["first_delay"] for r in results if r["first_delay"] > 0]
print(f"\n=== TIMING ===")
print(f"Avg first entry: {sum(delays)/len(delays):.0f}s into window" if delays else "N/A")
print(f"Min first entry: {min(delays)}s" if delays else "")
print(f"Max first entry: {max(delays)}s" if delays else "")

# Entry prices
entries = [r["avg_entry"] for r in results]
print(f"\n=== ENTRY PRICES ===")
print(f"Avg entry: {sum(entries)/len(entries):.3f}")
print(f"Min entry: {min(entries):.3f}")
print(f"Max entry: {max(entries):.3f}")

# Capital per trade
caps = [r["buy_usdc"] for r in results]
print(f"\n=== CAPITAL PER TRADE ===")
print(f"Avg: ${sum(caps)/len(caps):.2f}")
print(f"Min: ${min(caps):.2f}")
print(f"Max: ${max(caps):.2f}")
print(f"Median: ${sorted(caps)[len(caps)//2]:.2f}")

# Per-asset/timeframe
for a in ["BTC", "ETH", "SOL", "XRP"]:
    for tf in ["5m", "15m"]:
        subset = [r for r in results if r["asset"] == a and r["tf"] == tf]
        if not subset: continue
        w = len([r for r in subset if r["outcome"] == "WIN"])
        l = len([r for r in subset if r["outcome"] == "LOSS"])
        p = sum(r["pnl"] for r in subset)
        print(f"\n{a} {tf}: {len(subset)} mkts | W:{w} L:{l} | WR:{w/len(subset)*100:.0f}% | PnL:${p:.2f}")

# Frequency
all_ts = []
for r in results:
    parts = r["slug"].split("-")
    try: all_ts.append(int(parts[-1]))
    except: pass
if all_ts:
    all_ts.sort()
    hours = (all_ts[-1] - all_ts[0]) / 3600
    print(f"\n=== FREQUENCY ===")
    print(f"Active window: {hours:.1f} hours")
    print(f"Markets per hour: {len(results)/hours:.1f}")
    btc5 = [r for r in results if r["asset"] == "BTC" and r["tf"] == "5m"]
    possible_btc5 = hours * 12
    print(f"BTC 5m played: {len(btc5)} / {possible_btc5:.0f} possible ({len(btc5)/possible_btc5*100:.0f}%)")

# Exit method breakdown
sold_only = [r for r in results if r["sell_usdc"] > 0 and r["redeem_usdc"] == 0]
redeemed_only = [r for r in results if r["redeem_usdc"] > 0 and r["sell_usdc"] == 0]
both = [r for r in results if r["sell_usdc"] > 0 and r["redeem_usdc"] > 0]
neither = [r for r in results if r["sell_usdc"] == 0 and r["redeem_usdc"] == 0]
print(f"\n=== EXIT METHOD ===")
print(f"SELL only (scalped before settlement): {len(sold_only)}")
print(f"REDEEM only (held to settlement win): {len(redeemed_only)}")
print(f"SELL + REDEEM (partial exits): {len(both)}")
print(f"Neither (total loss, no exit): {len(neither)}")

# Show 5 biggest wins and 5 biggest losses
results.sort(key=lambda x: x["pnl"], reverse=True)
print(f"\n=== TOP 5 WINS ===")
for r in results[:5]:
    print(f"  {r['asset']} {r['tf']} | Dir:{r['direction']} | Entry:{r['avg_entry']:.3f} | ${r['buy_usdc']:.0f}->${r['sell_usdc']+r['redeem_usdc']:.0f} | P&L:+${r['pnl']:.2f} | Delay:{r['first_delay']}s | {r['num_buys']}buys/{r['num_sells']}sells")

print(f"\n=== WORST 5 LOSSES ===")
for r in results[-5:]:
    print(f"  {r['asset']} {r['tf']} | Dir:{r['direction']} | Entry:{r['avg_entry']:.3f} | ${r['buy_usdc']:.0f}->${r['sell_usdc']+r['redeem_usdc']:.0f} | P&L:${r['pnl']:.2f} | Delay:{r['first_delay']}s | {r['num_buys']}buys/{r['num_sells']}sells")
