import requests
import json
import time
import sys
import argparse
from collections import defaultdict

# USER CONFIG
API_KEY = "5P1T5B2K9EY1H789JEPIU9F5D23GQAU6K7"
PROXY_CONTRACT = "0x4d97dcd97ec945f40cf65f87097ace5ea0476045"
USDC_CONTRACT = "0x2791bca1f2de4661ed88a30c99a7a9449aa84174".lower()

def get_recent_traders(limit=2000):
    """Fetch recent active traders from the Polymarket Proxy contract."""
    url = f"https://api.etherscan.io/v2/api?chainid=137&module=account&action=txlist&address={PROXY_CONTRACT}&startblock=0&endblock=99999999&page=1&offset={limit}&sort=desc&apikey={API_KEY}"
    try:
        r = requests.get(url)
        res = r.json()
        if res.get("status") != "1":
            print(f"\nEtherscan Error: {res.get('status')} - {res.get('message')} - {res.get('result')}")
            return []
        txs = res.get("result", [])
        traders = set()
        for tx in txs:
            trader = tx.get("from", "").lower()
            if trader and trader != PROXY_CONTRACT:
                traders.add(trader)
        return list(traders)
    except Exception as e:
        print(f"\nFetch error: {e}")
        return []

def get_current_balance(address):
    """Fetch current USDC.e balance from Etherscan (Fast)."""
    url = f"https://api.etherscan.io/v2/api?chainid=137&module=account&action=tokenbalance&contractaddress={USDC_CONTRACT}&address={address}&tag=latest&apikey={API_KEY}"
    try:
        r = requests.get(url)
        data = r.json()
        if data.get("status") == "1":
            return float(data.get("result", 0)) / 1e6
        return 0.0
    except:
        return 0.0

def get_initial_funding(address):
    """Check initial funding for basic filtering (Shallow check)."""
    url = f"https://api.etherscan.io/v2/api?chainid=137&module=account&action=tokentx&address={address}&startblock=0&endblock=99999999&page=1&offset=50&sort=asc&apikey={API_KEY}"

    try:
        r = requests.get(url)
        data = r.json()
        if data.get("status") != "1": return 999999
        transfers = data.get("result", [])
        if not transfers: return 0
        external_in = 0.0
        for tx in transfers[:10]: # Check first 10 transfers for initial funding
            if tx.get("contractAddress").lower() == USDC_CONTRACT and tx.get("to").lower() == address.lower():
                external_in += float(tx.get("value", 0)) / 1e6
        return external_in
    except:
        return 999999

def analyze_trader(address, min_trades, max_avg_bet, min_win_rate, min_participation):
    """Analyze trader behavior for specialization, win rate, and participation."""
    url = f"https://data-api.polymarket.com/activity?user={address}&limit=500"
    try:
        resp = requests.get(url)
        if resp.status_code != 200: return None
        data = resp.json()
        if not data: return None
    except:
        return None
    
    # Filter for MUST HAVE crypto 5m/15m "updown" markets
    crypto_short = []
    for d in data:
        slug = str(d.get('eventSlug', '')).lower()
        title = str(d.get('title', '')).lower()
        if ("5m" in slug or "15m" in slug) and ("btc" in slug or "eth" in slug or "sol" in slug):
            if "updown" in slug or "price of" in title:
                crypto_short.append(d)
    
    if len(crypto_short) < min_trades: 
        return None
        
    markets = defaultdict(lambda: {"in": 0.0, "out": 0.0, "ts": 0, "side": ""})
    for e in crypto_short:
        slug = e.get("eventSlug") or e.get("title", "unknown")
        type_ = e.get("type", "UNKNOWN")
        usdc = float(e.get("usdcSize", 0))
        ts = e.get("timestamp", 0)
        outcome = e.get("outcome") # YES/NO
        
        if type_ == "TRADE":
            if e.get("side") == "BUY":
                markets[slug]["in"] += usdc
                markets[slug]["ts"] = ts
                markets[slug]["side"] = outcome
            else:
                markets[slug]["out"] += usdc
        elif type_ == "REDEEM":
            markets[slug]["out"] += usdc

    if not markets: return None

    # PARTICIPATION RATE CALCULATION
    sorted_ts = sorted([m["ts"] for m in markets.values() if m["ts"] > 0])
    if len(sorted_ts) < 5: return None
    
    span = sorted_ts[-1] - sorted_ts[0]
    possible_windows = (span / 300) + 1
    actual_windows = len(markets)
    participation_rate = (actual_windows / possible_windows) * 100 if possible_windows > 0 else 0
    
    if participation_rate < min_participation:
        return None

    wins, losses = 0, 0
    total_in, total_out = 0.0, 0.0
    for m in markets.values():
        if m["in"] > 0:
            pnl = m["out"] - m["in"]
            if pnl > 0.02: wins += 1
            elif pnl < -0.02: losses += 1
            total_in += m["in"]
            total_out += m["out"]
            
    total_played = wins + losses
    if total_played < 5: return None
    
    win_rate = (wins / total_played) * 100
    avg_bet = total_in / total_played
    
    if win_rate >= min_win_rate and avg_bet <= max_avg_bet and (total_out - total_in) > 0:
        return {
            "address": address,
            "win_rate": win_rate,
            "avg_bet": avg_bet,
            "total_pnl": total_out - total_in,
            "markets": total_played,
            "participation": participation_rate
        }
    return None

def main():
    parser = argparse.ArgumentParser(description="Polymarket Bot Scanner - Consolidate Analysis")
    parser.add_argument("--limit", type=int, default=2000, help="Number of Etherscan transactions to scan")
    parser.add_argument("--min-trades", type=int, default=20, help="Minimum trades in specialty to consider")
    parser.add_argument("--max-avg-bet", type=float, default=50.0, help="Maximum average bet size")
    parser.add_argument("--min-wr", type=float, default=52.0, help="Minimum win rate percentage")
    parser.add_argument("--min-part", type=float, default=5.0, help="Minimum participation rate percentage")
    parser.add_argument("--max-bankroll", type=float, default=100.0, help="Maximum initial funding (shallow check)")
    
    args = parser.parse_args()

    print(f"--- BOT SCANNER STARTING ---")
    print(f"Filters: Min WR: {args.min_wr}% | Max Avg Bet: ${args.max_avg_bet} | Min Partic: {args.min_part}%")
    
    traders = get_recent_traders(args.limit)
    print(f"Found {len(traders)} unique active traders from the last {args.limit} Proxy txs.")
    
    found = []
    for i, t in enumerate(traders):
        print(f"[{i+1}/{len(traders)}] Scanning {t}...", end="\r")
        
        res = analyze_trader(t, args.min_trades, args.max_avg_bet, args.min_wr, args.min_part)
        if not res:
            continue
            
        # 2. WHALE CHECK (SAFETY NET)
        current_bal = get_current_balance(t)
        if current_bal > 100000: # Only skip massive institutional-level wallets (>$100k)
            continue


        # 3. BANKROLL START CHECK (SHALLOW)
        cash = get_initial_funding(t)
        if cash > args.max_bankroll:
            continue
            
        res["current_balance"] = current_bal
        res["starting_cash"] = cash

        found.append(res)
        print(f"\n[!] CANDIDATE FOUND: {t}")
        print(f"    Started with: ${cash:.2f} (approx)")
        print(f"    Win Rate: {res['win_rate']:.1f}% over {res['markets']} markets")
        print(f"    Avg Bet: ${res['avg_bet']:.2f} | Participation: {res['participation']:.1f}%")
        print(f"    PnL in Sample: +${res['total_pnl']:.2f}")
        
    print("\n\n--- DISCOVERY COMPLETE ---")
    print(f"Saved {len(found)} candidates to shortlist.")

if __name__ == "__main__":
    main()
