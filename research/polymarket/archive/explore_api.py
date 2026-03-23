import requests
import json

token_id = "52454970267659310020729069966755673145772878749491"
url = f"https://clob.polymarket.com/trades?market={token_id}"
resp = requests.get(url)

if resp.status_code == 200:
    data = resp.json()
    print("Keys in response:", data.keys() if isinstance(data, dict) else type(data))
    if isinstance(data, dict) and "data" in data:
        trades = data["data"]
        print(f"Got {len(trades)} trades.")
        if trades:
            print(trades[0])
    elif isinstance(data, list) and len(data) > 0:
        print(f"Got {len(data)} trades.")
        print(data[0])
else:
    print(f"Failed: {resp.status_code} {resp.text}")


