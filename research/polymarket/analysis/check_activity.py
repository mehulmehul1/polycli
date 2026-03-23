import requests
import json

def check_bot_activity(address):
    url = f"https://data-api.polymarket.com/activity?user={address}&limit=50"
    try:
        resp = requests.get(url)
        data = resp.json()
        print(f"--- ACTIVITY FOR {address} ---")
        for i, event in enumerate(data):
            type_ = event.get("type")
            slug = event.get("eventSlug")
            side = event.get("side")
            price = event.get("price")
            usdc = event.get("usdcSize")
            outcome = event.get("outcome")
            ts = event.get("timestamp")
            
            if type_ == "TRADE":
                print(f"[{i}] {type_} | {side} {outcome} @ {price} | ${usdc} | {slug}")
    except Exception as e:
        print(f"Error: {e}")

if __name__ == "__main__":
    # 0x241a... (98% WR, -$41 PnL)
    check_bot_activity("0x241a15dc3a00237134d333a7b1bb9425a5689b06")
    print("\n" + "="*50 + "\n")
    # 0x6e79... (97% WR, +$36 PnL)
    check_bot_activity("0x6e7999562921b1769ae7fbe920db45416b950658")
