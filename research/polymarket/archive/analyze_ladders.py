import json
import requests
import time

address = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"
url = f"https://data-api.polymarket.com/activity?user={address}&limit=100"

all_events = []
next_cursor = None

for i in range(10): # Just 1000 events to sample
    req_url = url
    if next_cursor:
        req_url += f"&cursor={next_cursor}"
    resp = requests.get(req_url)
    if resp.status_code != 200: break
    data = resp.json()
    if not data: break
    all_events.extend(data)
    if len(data) < 100: break
    next_cursor = data[-1].get("timestamp")
    time.sleep(0.2)

# Find all events where user bought something. Group by eventSlug.
markets = {}

for e in all_events:
    t = e.get("type")
    side = e.get("side")
    if t == "TRADE" and side == "BUY":
        slug = e.get("eventSlug")
        if slug not in markets:
            markets[slug] = []
        markets[slug].append(e)

print(f"Sampled {len(markets)} distinct markets heavily traded.")

# Calculate average entry spacing
ladder_spacings = []
lowest_entries = []

for slug, trades in markets.items():
    if len(trades) < 2:
        continue
    
    # Sort trades by time
    trades.sort(key=lambda x: x.get("timestamp"))
    prices = [float(t.get("price")) for t in trades]
    
    # only look at ones where they laddered DOWN
    descending = True
    for i in range(1, len(prices)):
        if prices[i] > prices[i-1]:
            descending = False
            break
            
    if descending and len(prices) >= 2:
        spacing = prices[0] - prices[-1]
        ladder_spacings.append(spacing)
        lowest_entries.append(prices[-1])

print(f"Found {len(ladder_spacings)} clear ladder down examples.")
if ladder_spacings:
    print(f"Average ladder depth (top entry to bottom entry): {sum(ladder_spacings)/len(ladder_spacings):.3f}")
    print(f"Average absolute lowest entry price: {sum(lowest_entries)/len(lowest_entries):.3f}")
    print(f"Lowest entry price ever taken: {min(lowest_entries):.3f}")
