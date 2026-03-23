import requests
import json
import time
from collections import defaultdict

address = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"

all_events = []
next_cursor = None

print("Fetching ALL polymarket activity to find funding...")
for i in range(100):  # Should be enough for a 1-2 day old wallet (10k events max here)
    url = f"https://data-api.polymarket.com/activity?user={address}&limit=100"
    if next_cursor:
        url += f"&cursor={next_cursor}"
    resp = requests.get(url)
    if resp.status_code != 200: 
        break
    data = resp.json()
    if not data: 
        break
    all_events.extend(data)
    if len(data) < 100: 
        break
    next_cursor = data[-1].get("timestamp")
    time.sleep(0.1)

print(f"Total events fetched: {len(all_events)}")

# Find all events that represent funding (DEPOSIT, WITHDRAW, etc.)
# Identify all types of events first
event_types = set([e.get("type") for e in all_events])
print("Event types found:", event_types)

deposits = 0
withdrawals = 0

for e in all_events:
    t = e.get("type")
    
    # Polymarket activity types: DEPOSIT, WITHDRAW, TRANSFER, etc
    if t == "DEPOSIT":
        val = float(e.get("amount", 0)) if "amount" in e else float(e.get("usdcSize", 0))
        if val == 0 and "size" in e: val = float(e.get("size"))
        deposits += val
        print(f"DEPOSIT: +${val:.2f}")
    
    elif t == "WITHDRAW" or t == "WITHDRAWAL":
        val = float(e.get("amount", 0)) if "amount" in e else float(e.get("usdcSize", 0))
        if val == 0 and "size" in e: val = float(e.get("size"))
        withdrawals += val
        print(f"WITHDRAWAL: -${val:.2f}")

print("-" * 40)
print(f"Total Deposits   : ${deposits:.2f}")
print(f"Total Withdrawals: ${withdrawals:.2f}")
print(f"Net Funded       : ${deposits - withdrawals:.2f}")

# Also calculate net cash flow strictly from trades + redeems (ignoring fees/rewards for a bit)
buy_vol = sum(float(e.get("usdcSize", 0)) for e in all_events if e.get("type") == "TRADE" and e.get("side") == "BUY")
sell_vol = sum(float(e.get("usdcSize", 0)) for e in all_events if e.get("type") == "TRADE" and e.get("side") == "SELL")
redeem_vol = sum(float(e.get("usdcSize", 0)) for e in all_events if e.get("type") == "REDEEM")

print("\n--- TRADING CASH FLOW ---")
print(f"Total Bought : ${buy_vol:.2f}")
print(f"Total Sold   : ${sell_vol:.2f}")
print(f"Total Redeem : ${redeem_vol:.2f}")
print(f"Cash PnL     : ${(sell_vol + redeem_vol) - buy_vol:.2f}")

