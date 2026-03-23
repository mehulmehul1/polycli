import requests
import json

address = "0xa74330685830ab52bafe35400fae7c9c100725d8"
url = f"https://data-api.polymarket.com/activity?user={address}&limit=20"
resp = requests.get(url)
data = resp.json()

for d in data:
    slug = d.get('eventSlug', 'N/A')
    title = d.get('title', 'N/A')
    ts = d.get('timestamp')
    type_ = d.get('type')
    side = d.get('side', '')
    usdc = d.get('usdcSize')
    
    # Determine window start if possible
    window_start = 0
    try:
        window_start = int(slug.split("-")[-1])
    except:
        pass
        
    delay = ts - window_start if window_start > 0 else "N/A"
    
    print(f"TS: {ts} | Delay: {delay}s | Type: {type_} {side} | USDC: {usdc} | Slug: {slug}")
    print(f"  Title: {title}\n")
