import requests
import json
import time
import sys

# Read traders
try:
    with open('traders.txt', 'r') as f:
        traders = [line.strip() for line in f if line.strip()]
except:
    print("No traders.txt found.")
    sys.exit(1)

print(f"Scanning {len(traders)} traders for 5m BTC activity...")

found_candidates = []

for idx, t in enumerate(traders):
    # Just fetch their most recent 50 activities to see if they trade 5m BTC
    url = f"https://data-api.polymarket.com/activity?user={t}&limit=50"
    try:
        resp = requests.get(url)
        if resp.status_code != 200:
            time.append(0.2)
            continue
        data = resp.json()
    except:
        continue
        
    btc_5m_trades = [d for d in data if "5m" in str(d.get('title', '')).lower() or "btc" in str(d.get('title', '')).lower()]
    
    if len(btc_5m_trades) >= 5: # Need a good sample
        # Calculate avg bet size to ensure it's small bankroll
        bet_sizes = [float(d.get('usdcSize', 0)) for d in btc_5m_trades if d.get('type') == 'TRADE' and float(d.get('usdcSize', 0)) > 0]
        if not bet_sizes:
            time.sleep(0.3)
            continue
            
        avg_bet = sum(bet_sizes) / len(bet_sizes)
        
        # We want small bankroll ($1 - $25)
        if 0.5 <= avg_bet <= 30.0:
            print(f"\n[{idx+1}/{len(traders)}] 🚨 CANDIDATE FOUND: {t}")
            print(f"   -> Average 5m BTC Bet Size: ${avg_bet:.2f} USDC")
            print(f"   -> Recent 5m BTC Activity Count: {len(btc_5m_trades)}")
            found_candidates.append(t)
            
    time.sleep(0.2) # PolyMarket rate limit is generous but let's be safe
    sys.stdout.write(f"\rProgress: {idx+1}/{len(traders)} scanned. Found {len(found_candidates)}.")
    sys.stdout.flush()

print("\n\n--- SCAN COMPLETE ---")
print("Top candidates:")
for c in found_candidates:
    print(c)
    
if not found_candidates:
    print("No small-bankroll 5m BTC traders found in this recent batch. We may need to pull a larger list of addresses.")
