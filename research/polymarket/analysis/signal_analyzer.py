import requests
import json
import time
import sys
import argparse
from collections import defaultdict

def get_activity(user, limit=500):
    url = f"https://data-api.polymarket.com/activity?user={user}&limit={limit}"
    r = requests.get(url)
    if r.status_code != 200: return []
    return r.json()

def analyze_signals(address, limit=500):
    print(f"--- SIGNAL ANALYSIS for {address} ---")
    data = get_activity(address, limit)
    trades = [d for d in data if d.get('type') == 'TRADE' and d.get('side') == 'BUY']

    if not trades:
        print("No BUY trades found in the recent sample.")
        return

    print(f"Analyzing {len(trades)} recent BUY trades...")

    results = defaultdict(list)
    for t in trades:
        slug = t.get('eventSlug', '')
        ts = t.get('timestamp')
        usdc = t.get('usdcSize')
        outcome = t.get('outcome')
        
        try:
            parts = slug.split("-")
            # Expected slug format: btc-updown-5m-1773456900
            if len(parts) < 4: continue
            
            interval = parts[2] # 5m or 15m
            window_start = int(parts[-1])
            delay = ts - window_start
            
            entry = {"delay": delay, "size": usdc, "side": outcome, "slug": slug}
            results[interval].append(entry)
        except Exception as e:
            continue

    for interval, res in results.items():
        if not res: continue
        delays = [r['delay'] for r in res]
        avg = sum(delays) / len(delays)
        
        print(f"\n--- {interval.upper()} WINDOW STATS ({len(res)} trades) ---")
        print(f"  Avg Delay: {avg:.1f}s | Min: {min(delays)}s | Max: {max(delays)}s")
        
        # Identify strategy profile
        if interval == "5m":
            if avg > 301:
                profile = "POST-EXPIRY ARB (Latency Pickoff)"
            elif avg > 270:
                profile = "LATE-WINDOW MOMENTUM / SNIPING"
            elif avg < 60:
                profile = "EARLY-WINDOW GAP / NEWS ARB"
            else:
                profile = "MID-WINDOW DIRECTIONAL (Trend)"
        elif interval == "15m":
            if avg > 901:
                profile = "POST-EXPIRY ARB (Latency Pickoff)"
            elif avg > 840:
                profile = "LATE-WINDOW MOMENTUM / SNIPING"
            elif avg < 120:
                profile = "EARLY-WINDOW GAP / NEWS ARB"
            else:
                profile = "MID-WINDOW DIRECTIONAL (Trend)"
        else:
            profile = "UNKNOWN (Custom Window)"

        print(f"  [!] DETECTED PROFILE: {profile}")


        # Show first 5 examples
        print("  Recent Samples:")
        for r in res[:5]:
            print(f"    {r['slug']} | +{r['delay']}s | ${r['size']} | {r['side']}")

def main():
    parser = argparse.ArgumentParser(description="Polymarket Signal & Timing Analyzer")
    parser.add_argument("address", help="Wallet address to analyze")
    parser.add_argument("--limit", type=int, default=500, help="Number of activity items to sample")
    
    args = parser.parse_args()
    analyze_signals(args.address.lower(), args.limit)

if __name__ == "__main__":
    main()
