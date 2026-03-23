import requests
import time
import sys

address = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"
url = f"https://data-api.polymarket.com/activity?user={address}&limit=100"

all_events = []
next_cursor = None

for i in range(10):  # Fetch up to 1000 events
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
    
    # Try to find next cursor from last item or headers if polymarket uses that
    if len(data) < 100:
        break
    next_cursor = data[-1].get("timestamp") # polymorphic often paginates by timestamp
    time.sleep(0.5)

print(f"Total events found: {len(all_events)}")
if all_events:
    print(f"First event date: {all_events[-1].get('timestamp')}")
    print(f"First event type: {all_events[-1].get('type')}")
    
    # Let's sum up deposits vs withdrawals (if polymarket tags them)
    # Usually they tag USDC transfers or just "TRADE" / "REDEEM"
    trades = [e for e in all_events if e.get("type") == "TRADE"]
    print(f"Total trades: {len(trades)}")
    
    if trades:
        print(f"Earliest trade: {trades[-1]}")
