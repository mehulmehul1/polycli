import requests
import json
import time
import sys
from collections import defaultdict
from datetime import datetime

# USER CONFIG
API_KEY = "5P1T5B2K9EY1H789JEPIU9F5D23GQAU6K7"
USDC_CONTRACT = "0x2791bca1f2de4661ed88a30c99a7a9449aa84174".lower()

def fetch_full_onchain_history(user_address):
    """Fetch complete USDC.e transfer history using full pagination."""
    print(f"Fetching full USDC token transfer history for {user_address}...")
    transfers = []
    page = 1
    offset = 1000
    
    while True:
        url = f"https://api.etherscan.io/v2/api?chainid=137&module=account&action=tokentx&address={user_address}&startblock=0&endblock=99999999&page={page}&offset={offset}&sort=asc&apikey={API_KEY}"
        try:
            r = requests.get(url)
            data = r.json()
            if data.get("status") != "1":
                if data.get("message") == "No transactions found":
                    break
                print(f"Etherscan pagination ended or errored: {data.get('message')}")
                break
            
            result = data.get("result", [])
            # Filter only USDC.e
            usdc_only = [t for t in result if t.get("contractAddress").lower() == USDC_CONTRACT]
            transfers.extend(usdc_only)
            
            print(f"  Page {page}: Found {len(usdc_only)} USDC transfers (Total: {len(transfers)})")
            
            if len(result) < offset:
                break
            page += 1
            time.sleep(0.1) # Respect API limits
        except Exception as e:
            print(f"Error during on-chain fetch: {e}")
            break
            
    return transfers

def fetch_full_polymarket_activity(user_address):
    """Fetch all activity from Polymarket Data API using cursor-based pagination."""
    print(f"Fetching full Polymarket activity for {user_address}...")
    all_events = []
    next_cursor = None
    
    while True:
        url = f"https://data-api.polymarket.com/activity?user={user_address}&limit=500"
        if next_cursor:
            url += f"&cursor={next_cursor}"
        
        try:
            resp = requests.get(url)
            if resp.status_code != 200: break
            data = resp.json()
            if not data: break
            
            all_events.extend(data)
            print(f"  Fetched {len(all_events)} events...")
            
            if len(data) < 500: break
            next_cursor = data[-1].get("timestamp")
            time.sleep(0.1)
        except Exception as e:
            print(f"Error during PM activity fetch: {e}")
            break
            
    return all_events

def run_pnl_audit(address):
    print(f"\n--- AUDIT START: {address} ---")
    
    onchain_transfers = fetch_full_onchain_history(address)
    pm_activity = fetch_full_polymarket_activity(address)
    
    if not onchain_transfers:
        print("No USDC transfers found. Is this a new or custom proxy wallet?")
    else:
        # Bankroll Analysis
        print(f"\n[1] BANKROLL GROWTH AUDIT")
        # Sort by timestamp just in case
        onchain_transfers.sort(key=lambda x: int(x.get("timeStamp")))
        
        first_tx = onchain_transfers[0]
        initial_val = float(first_tx.get("value", 0)) / 1e6
        initial_ts = datetime.fromtimestamp(int(first_tx.get("timeStamp")))
        
        balance = 0.0
        max_balance = 0.0
        for t in onchain_transfers:
            val = float(t.get("value", 0)) / 1e6
            if t.get("to").lower() == address.lower():
                balance += val
            else:
                balance -= val
            max_balance = max(max_balance, balance)
            
        print(f"  First Funding: ${initial_val:.2f} on {initial_ts}")
        print(f"  Current Realized Balance: ${balance:.2f}")
        print(f"  Peak Balance (Realized): ${max_balance:.2f}")
        
    if not pm_activity:
        print("No Polymarket activity found.")
    else:
        print(f"\n[2] PERFORMANCE METRICS")
        markets = defaultdict(lambda: {"in": 0.0, "out": 0.0, "win": 0, "loss": 0, "active": True})
        
        for e in pm_activity:
            slug = e.get("eventSlug") or e.get("title")
            val = float(e.get("usdcSize", 0))
            type_ = e.get("type")
            
            if type_ == "TRADE":
                if e.get("side") == "BUY":
                    markets[slug]["in"] += val
                else:
                    markets[slug]["out"] += val
            elif type_ == "REDEEM":
                markets[slug]["out"] += val
        
        total_in = 0.0
        total_out = 0.0
        wins = 0
        losses = 0
        
        for slug, m in markets.items():
            if m["in"] > 0:
                total_in += m["in"]
                total_out += m["out"]
                profit = m["out"] - m["in"]
                if profit > 0.05: wins += 1
                elif profit < -0.05: losses += 1
        
        win_rate = (wins / (wins + losses) * 100) if (wins + losses) > 0 else 0
        net_profit = total_out - total_in
        
        print(f"  Total Trades Grouped: {len(markets)}")
        print(f"  Net PnL (Calculated): ${net_profit:.2f}")
        print(f"  Win Rate: {win_rate:.1f}% ({wins}W / {losses}L)")
        print(f"  Total Volume: ${total_in:.2f}")

    print(f"\n--- AUDIT COMPLETE ---")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python pnl_verifier.py <wallet_address>")
        sys.exit(1)
    
    target_address = sys.argv[1].lower()
    run_pnl_audit(target_address)
