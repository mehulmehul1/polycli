import requests
import time

address = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"
url = f"https://data-api.polymarket.com/activity?user={address}&limit=100"

all_events = []
next_cursor = None

for i in range(50):  # Fetch up to 5000 events to ensure we get everything
    req_url = url
    if next_cursor:
        req_url += f"&cursor={next_cursor}"
    
    resp = requests.get(req_url)
    if resp.status_code != 200:
        break
        
    data = resp.json()
    if not data:
        break
        
    all_events.extend(data)
    
    if len(data) < 100:
        break
    next_cursor = data[-1].get("timestamp")
    time.sleep(0.2)

total_buys = 0.0
total_sells = 0.0
total_redeems = 0.0

for e in all_events:
    t = e.get("type")
    usdc = float(e.get("usdcSize", 0) or 0)
    if t == "TRADE":
        if e.get("side") == "BUY":
            total_buys += usdc
        elif e.get("side") == "SELL":
            total_sells += usdc
    elif t == "REDEEM":
        total_redeems += usdc

current_value_resp = requests.get(f"https://data-api.polymarket.com/value?user={address}")
current_value = 0.0
if current_value_resp.status_code == 200:
    cv_data = current_value_resp.json()
    if cv_data and len(cv_data) > 0:
        current_value = float(cv_data[0].get("value", 0))

print(f"Total Events: {len(all_events)}")
if all_events:
    print(f"First event time: {all_events[-1].get('timestamp')}")

print(f"Total spent on BUYs: ${total_buys:.2f}")
print(f"Total returned from SELLs: ${total_sells:.2f}")
print(f"Total returned from REDEEMs: ${total_redeems:.2f}")
print(f"Current Portfolio Value: ${current_value:.2f}")

actual_pnl = (total_sells + total_redeems + current_value) - total_buys
print(f"ACTUAL TRUE PNL: ${actual_pnl:.2f}")

