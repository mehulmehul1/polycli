import requests
import json
import time
import sys
from collections import defaultdict

api_key = "5P1T5B2K9EY1H789JEPIU9F5D23GQAU6K7"
proxy = "0x4d97dcd97ec945f40cf65f87097ace5ea0476045"
etherscan_url = f"https://api.etherscan.io/v2/api?chainid=137&module=account&action=txlist&address={proxy}&startblock=0&endblock=999999999&page=1&offset=500&sort=desc&apikey={api_key}"

print("1. Fetching recent traders from Proxy contract...")
r = requests.get(etherscan_url)
txs = r.json().get('result', [])
traders = set()
for tx in txs:
    trader = tx.get('from', '').lower()
    if trader and trader != proxy:
        traders.add(trader)
print(f"Found {len(traders)} unique active traders.")

print("\n2. Finding PROFITABLE, SMALL-BANKROLL 5m BTC traders...")
found_candidates = []

for idx, t in enumerate(list(traders)):
    url = f"https://data-api.polymarket.com/activity?user={t}&limit=300"
    try:
        resp = requests.get(url)
        if resp.status_code != 200:
            time.sleep(0.2)
            continue
        data = resp.json()
    except:
        continue
        
    # Filter 5m BTC activities
    btc_5m = [d for d in data if "5m" in str(d.get('title', '')).lower() or "btc" in str(d.get('title', '')).lower()]
    if len(btc_5m) < 10: 
        time.sleep(0.3)
        continue
        
    markets = defaultdict(lambda: {"buy_usdc": 0.0, "return_usdc": 0.0, "trades": 0})
    
    for e in btc_5m:
        slug = e.get("eventSlug") or e.get("title", "unknown")
        type_ = e.get("type", "UNKNOWN")
        side = e.get("side", "")
        usdc = float(e.get("usdcSize", 0))
        
        if type_ == "TRADE" and side == "BUY":
            markets[slug]["buy_usdc"] += usdc
            markets[slug]["trades"] += 1
        elif type_ == "TRADE" and side == "SELL":
            markets[slug]["return_usdc"] += usdc
        elif type_ == "REDEEM":
            markets[slug]["return_usdc"] += usdc

    wins = 0
    losses = 0
    total_profit = 0.0
    total_bet = 0.0
    
    for slug, m in markets.items():
        if m["buy_usdc"] > 0:
            pnl = m["return_usdc"] - m["buy_usdc"]
            if pnl > 0.1: wins += 1
            elif pnl < -0.1: losses += 1
            total_profit += pnl
            total_bet += m["buy_usdc"]
            
    total_markets = wins + losses
    if total_markets < 3: 
        time.sleep(0.3)
        continue
        
    avg_bet = total_bet / len(markets) if markets else 0
    win_rate = wins / total_markets * 100
    
    # CRITERIA:
    # 1. Profitable (Total Profit > 0)
    # 2. High Win Rate (> 55%)
    # 3. Small Bankroll ($1 to $50 average bet)
    if total_profit > 0 and win_rate >= 50.0 and 1.0 <= avg_bet <= 50.0:
        print(f"\n\n🚨 WINNER FOUND: {t}")
        print(f" -> Markets Played : {total_markets}")
        print(f" -> Win Rate       : {win_rate:.1f}%")
        print(f" -> Total Net PnL  : +${total_profit:.2f}")
        print(f" -> Avg Bet Size   : ${avg_bet:.2f}")
        found_candidates.append(t)
        
    sys.stdout.write(f"\rProgress: {idx+1}/{len(traders)}... Found: {len(found_candidates)}")
    sys.stdout.flush()
    time.sleep(0.3)

print("\n\n--- SEARCH COMPLETE ---")
if not found_candidates:
    print("No small-bankroll, profitable 5m BTC bots found in this immediate sample.")
else:
    print("You can now run `python poly_reverse.py <address>` on any of these candidates!")
