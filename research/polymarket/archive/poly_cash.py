import requests

address = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"
all_events = []
next_cursor = None

for i in range(150):
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

print(f"Total events: {len(all_events)}")

buy_vol = sum(float(e.get("usdcSize", 0)) for e in all_events if e.get("type") == "TRADE" and e.get("side") == "BUY")
sell_vol = sum(float(e.get("usdcSize", 0)) for e in all_events if e.get("type") == "TRADE" and e.get("side") == "SELL")
redeem_vol = sum(float(e.get("usdcSize", 0)) for e in all_events if e.get("type") == "REDEEM")

print("\n--- EXACT CASH FLOW ---")
print(f"Total Bought : ${buy_vol:,.2f}")
print(f"Total Sold   : ${sell_vol:,.2f}")
print(f"Total Redeem : ${redeem_vol:,.2f}")
print(f"True Cash PnL: ${(sell_vol + redeem_vol) - buy_vol:,.2f}")
