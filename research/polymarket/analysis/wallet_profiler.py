import argparse
import sys
import time
from collections import defaultdict
from datetime import datetime

try:
    import requests
except ImportError:
    print("Please install requests: pip install requests")
    sys.exit(1)

# Change default encoding for Windows terminals
sys.stdout.reconfigure(encoding='utf-8')

# Constants
USDC_CONTRACT = "0x2791bca1f2de4661ed88a30c99a7a9449aa84174".lower()
CTF_EXCHANGE = "0x4bfb41d5b3570defd03c39a9a4d8de6bd8b8982e".lower()
POLYMARKET_PROXY = "0x4d97dcd97ec945f40cf65f87097ace5ea0476045".lower()


def fetch_onchain_transfers(address, api_key):
    """Fetch USDC.e transfer history with proper pagination."""
    print(f"\n[1/4] Fetching On-Chain USDC.e Transfers...")
    transfers = []
    page = 1
    offset = 1000

    while True:
        url = (
            f"https://api.etherscan.io/v2/api?chainid=137"
            f"&module=account&action=tokentx"
            f"&address={address}"
            f"&startblock=0&endblock=99999999"
            f"&page={page}&offset={offset}"
            f"&sort=asc&apikey={api_key}"
        )

        try:
            resp = requests.get(url)
            if resp.status_code != 200:
                print(f"Failed to fetch from Polygonscan: {resp.status_code}")
                break

            data = resp.json()

            if data.get("status") != "1":
                msg = data.get("message", "")
                result = data.get("result", "")

                if "No transactions found" in msg:
                    break
                elif "rate limit" in msg.lower() or "rate limit" in str(result).lower():
                    print("Hit API rate limit, waiting 2 seconds...")
                    time.sleep(2)
                    continue
                elif "Result window is too large" in str(result):
                    print(f"\n[WARNING] Whale Account: Hit 10,000 tx limit.")
                    print("Bankroll based on recent 10,000 transfers only.\n")
                    break
                else:
                    print(f"Etherscan Error: {msg} - {result}")
                    print("Skipping on-chain phase...\n")
                    break

            results = data.get("result", [])
            if not results or isinstance(results, str):
                break

            transfers.extend(results)
            print(f"  Page {page}: {len(results)} transfers (Total: {len(transfers)})")

            if len(results) < offset:
                break

            page += 1
            time.sleep(0.3)  # Respect Etherscan rate limits

        except Exception as e:
            print(f"Error during fetch: {e}")
            break

    return transfers


def fetch_polymarket_activity(address):
    """Fetch complete Polymarket activity using offset-based pagination."""
    print(f"\n[2/4] Fetching Polymarket Activity...")
    activity = []
    limit = 500
    offset = 0

    while True:
        url = f"https://data-api.polymarket.com/activity?user={address}&limit={limit}&offset={offset}"

        try:
            resp = requests.get(url)
            if resp.status_code != 200:
                print(f"Error hitting PM API: {resp.status_code}")
                break

            data = resp.json()
            if not data or not isinstance(data, list):
                break

            activity.extend(data)
            print(f"  Fetched {len(activity)} total events...")

            if len(data) < limit:
                break

            offset += limit
            time.sleep(0.1)

        except Exception as e:
            print(f"Error during PM fetch: {e}")
            break

    return activity


def analyze_strategy(activity, transfers):
    """Analyze trading strategy and persona."""
    print(f"\n[3/4] Analyzing Strategy & Execution...")

    # Build market-level statistics
    markets = defaultdict(lambda: {
        "buy_usdc": 0.0,
        "sell_usdc": 0.0,
        "redeem_usdc": 0.0,
        "sides": set(),
        "first_ts": None,
        "last_ts": None
    })

    for event in activity:
        slug = event.get("eventSlug") or event.get("title", "unknown")
        type_ = event.get("type")
        usdc = float(event.get("usdcSize", 0))
        ts = event.get("timestamp")

        if type_ == "TRADE":
            side = event.get("side")
            outcome = event.get("outcome", "")

            if side == "BUY":
                markets[slug]["buy_usdc"] += usdc
                markets[slug]["sides"].add(outcome)
                if markets[slug]["first_ts"] is None:
                    markets[slug]["first_ts"] = ts
            elif side == "SELL":
                markets[slug]["sell_usdc"] += usdc

        elif type_ == "REDEEM":
            markets[slug]["redeem_usdc"] += usdc

        if ts and markets[slug]["last_ts"] is None or (ts and ts > markets[slug]["last_ts"]):
            markets[slug]["last_ts"] = ts

    # Filter to markets with actual activity
    active_markets = {k: v for k, v in markets.items() if v["buy_usdc"] > 0}

    if not active_markets:
        print("No active markets found.")
        return None

    # Calculate metrics
    total_buy_usdc = sum(m["buy_usdc"] for m in active_markets.values())
    total_return_usdc = sum(
        m["sell_usdc"] + m["redeem_usdc"] for m in active_markets.values()
    )
    total_markets = len(active_markets)

    avg_bet_size = total_buy_usdc / total_markets if total_markets > 0 else 0

    # Calculate entry prices
    total_shares = 0.0
    weighted_price = 0.0
    for event in activity:
        if event.get("type") == "TRADE" and event.get("side") == "BUY":
            price = float(event.get("price", 0))
            shares = float(event.get("size", 0))
            if price > 0 and shares > 0:
                weighted_price += price * shares
                total_shares += shares

    avg_entry_price = weighted_price / total_shares if total_shares > 0 else 0

    # Calculate participation rate for updown markets
    updown_markets = {
        k: v for k, v in active_markets.items()
        if "updown" in k.lower() and ("5m" in k.lower() or "15m" in k.lower())
    }

    participation_rate = 0.0
    if len(updown_markets) >= 2:
        timestamps = sorted([
            m["first_ts"] for m in updown_markets.values()
            if m["first_ts"] is not None
        ])
        if timestamps:
            span = timestamps[-1] - timestamps[0]
            possible_windows = (span / 300) + 1  # 5-minute windows
            participation_rate = (len(updown_markets) / possible_windows * 100) if possible_windows > 0 else 0

    # Strategy classification
    straddle_count = sum(1 for m in active_markets.values() if len(m["sides"]) > 1)

    if straddle_count > total_markets * 0.3:
        strategy = "ARBITRAGE / STRADDLE"
        description = "Exploits mispricings where YES+NO sum < $1.00"
    elif avg_entry_price < 0.40:
        strategy = "DEEP DIP BUYING"
        description = f"Targets discounted assets (Avg Entry: ${avg_entry_price:.2f})"
    elif avg_entry_price > 0.60:
        strategy = "FAVORITE ACCUMULATION"
        description = f"Targets favorites (Avg Entry: ${avg_entry_price:.2f})"
    else:
        strategy = "DIRECTIONAL MOMENTUM"
        description = f"Mixed directional trading (Avg Entry: ${avg_entry_price:.2f})"

    print(f"  Total Markets Traded: {total_markets}")
    print(f"  Total Volume: ${total_buy_usdc:.2f}")
    print(f"  Avg Bet Size: ${avg_bet_size:.2f}")
    print(f"  Avg Entry Price: ${avg_entry_price:.3f}")
    if participation_rate > 0:
        print(f"  UpDown Participation: {participation_rate:.1f}%")

    return {
        "markets": active_markets,
        "total_markets": total_markets,
        "total_buy_usdc": total_buy_usdc,
        "total_return_usdc": total_return_usdc,
        "avg_bet_size": avg_bet_size,
        "avg_entry_price": avg_entry_price,
        "straddle_count": straddle_count,
        "strategy": strategy,
        "strategy_description": description,
        "participation_rate": participation_rate,
        "updown_markets": len(updown_markets)
    }


def analyze_bankroll(transfers, address):
    """Calculate on-chain bankroll metrics."""
    print(f"\n[4/4] Analyzing On-Chain Flows...")

    external_in, external_out = 0.0, 0.0
    ctf_in, ctf_out = 0.0, 0.0
    proxy_in, proxy_out = 0.0, 0.0

    usdc_transfers = [
        t for t in transfers
        if t.get("contractAddress", "").lower() == USDC_CONTRACT
    ]

    for tx in usdc_transfers:
        val = float(tx.get("value", 0)) / 1e6  # USDC has 6 decimals
        sender = tx.get("from", "").lower()
        receiver = tx.get("to", "").lower()
        target = address.lower()

        if receiver == target:
            if sender == CTF_EXCHANGE:
                ctf_in += val
            elif sender == POLYMARKET_PROXY:
                proxy_in += val
            else:
                external_in += val
        elif sender == target:
            if receiver == CTF_EXCHANGE:
                ctf_out += val
            elif receiver == POLYMARKET_PROXY:
                proxy_out += val
            else:
                external_out += val

    realized_bankroll = external_in - external_out
    total_pm_out = ctf_out + proxy_out
    total_pm_in = ctf_in + proxy_in
    net_pm_pnl = total_pm_in - total_pm_out

    print(f"  External Deposits: ${external_in:,.2f}")
    print(f"  External Withdrawals: ${external_out:,.2f}")
    print(f"  Realized Bankroll: ${realized_bankroll:,.2f}")
    print(f"  Polymarket Net PnL: ${net_pm_pnl:,.2f}")

    return {
        "external_in": external_in,
        "external_out": external_out,
        "realized_bankroll": realized_bankroll,
        "net_pm_pnl": net_pm_pnl
    }


def profile_wallet(address, api_key=None):
    """Main profiling function."""
    print(f"\n{'='*60}")
    print(f"  WALLET PROFILER: {address}")
    print(f"{'='*60}")

    # Use the working key from pnl_verifier.py
    if not api_key:
        api_key = "5P1T5B2K9EY1H789JEPIU9F5D23GQAU6K7"

    # Fetch data
    transfers = fetch_onchain_transfers(address, api_key)
    activity = fetch_polymarket_activity(address)

    if not transfers:
        print("\n[WARNING] No on-chain transfers found.")
        print("This may be a proxy wallet or the API key is invalid.\n")

    if not activity:
        print("\n[WARNING] No Polymarket activity found.\n")
        return

    # Analyze
    bankroll = analyze_bankroll(transfers, address) if transfers else None
    strategy = analyze_strategy(activity, transfers)

    if not strategy:
        return

    # Final synthesis
    print(f"\n{'='*60}")
    print(f"  STRATEGY PERSONA DETECTED")
    print(f"{'='*60}")
    print(f"  Classification: {strategy['strategy']}")
    print(f"  {strategy['strategy_description']}")
    print(f"\n  Top 5 Markets by Volume:")
    sorted_markets = sorted(
        strategy['markets'].items(),
        key=lambda x: x[1]['buy_usdc'],
        reverse=True
    )
    for slug, m in sorted_markets[:5]:
        short_slug = slug[:50] + "..." if len(slug) > 50 else slug
        print(f"    ${m['buy_usdc']:>8.2f}  {short_slug}")

    if bankroll and bankroll['realized_bankroll'] > 0:
        roi = (bankroll['net_pm_pnl'] / bankroll['realized_bankroll'] * 100)
        print(f"\n  ROI on Bankroll: {roi:+.1f}%")

    print(f"\n{'='*60}")


def main():
    parser = argparse.ArgumentParser(
        description="Polymarket Wallet Profiler - Strategy Persona Detection",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python wallet_profiler.py 0x9ce0cb7d0163551a7ad8cb5ffe1a821b54e9e9e8
  python wallet_profiler.py 0x9ce0... --api-key YOUR_KEY
        """
    )

    parser.add_argument(
        "address",
        help="Wallet address to profile"
    )
    parser.add_argument(
        "--api-key",
        default=None,
        help="Polygonscan API key (or set POLYGONSCAN_API_KEY env var)"
    )

    args = parser.parse_args()

    profile_wallet(args.address.lower(), args.api_key)


if __name__ == "__main__":
    main()
