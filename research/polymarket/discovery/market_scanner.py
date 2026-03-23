"""
Market-First Bot Scanner for Polymarket Crypto UpDown Markets

Instead of scanning the proxy contract randomly, this tool:
1. Generates market slugs for the last N hours of crypto updown markets
2. Queries Polymarket API for traders in each market
3. Aggregates unique addresses and filters for bot candidates

Slug pattern: {crypto}-updown-{5m|15m}-{unix_timestamp}
"""

import argparse
import json
import sys
import time
from collections import defaultdict
from datetime import datetime, timedelta
from typing import Set, List, Dict

try:
    import requests
except ImportError:
    print("Please install requests: pip install requests")
    sys.exit(1)

# Fix Windows console encoding
if sys.platform == "win32":
    try:
        sys.stdout.reconfigure(encoding='utf-8')
    except Exception:
        pass

# Constants
CRYPTOS = ["btc", "eth", "sol"]
INTERVALS = ["5m", "15m"]
BASE_URL = "https://polymarket.com/api"  # Will adjust based on actual API


def generate_market_slugs(hours_back: int = 24) -> List[str]:
    """
    Generate market slugs for the last N hours.

    5m markets: aligned to 5-min timestamps (divisible by 300)
    15m markets: aligned to 15-min timestamps (divisible by 900)
    """
    now = int(time.time())
    start_time = now - (hours_back * 3600)

    slugs = []

    # 5-minute intervals (300 seconds)
    first_5m = (start_time // 300) * 300
    for ts in range(first_5m, now, 300):
        for crypto in CRYPTOS:
            slugs.append(f"{crypto}-updown-5m-{ts}")

    # 15-minute intervals (900 seconds)
    first_15m = (start_time // 900) * 900
    for ts in range(first_15m, now, 900):
        for crypto in CRYPTOS:
            slugs.append(f"{crypto}-updown-15m-{ts}")

    return slugs


def get_market_condition_id(market_slug: str) -> str | None:
    """
    Fetch the condition ID for a market using its slug.

    Uses Polymarket's Gamma API to get market details.
    Returns condition ID as hex string (0x...) or None if not found.
    """
    try:
        url = f"https://gamma-api.polymarket.com/events?slug={market_slug}"
        resp = requests.get(url, timeout=10)
        if resp.status_code != 200:
            return None

        data = resp.json()
        if not data or not isinstance(data, list):
            return None

        # The first event should contain the market with conditionId
        event = data[0]
        markets = event.get("markets", [])
        if not markets:
            return None

        condition_id = markets[0].get("conditionId")
        if condition_id:
            # Ensure it has 0x prefix
            if isinstance(condition_id, str):
                return condition_id if condition_id.startswith("0x") else f"0x{condition_id}"

        return None

    except Exception as e:
        return None


def get_market_traders(market_slug: str) -> Set[str]:
    """
    Fetch unique trader addresses for a specific market.

    Two-step process:
    1. Get conditionId from market slug via Gamma API
    2. Get holders (traders) via Data API holders endpoint
    """
    traders = set()

    # Step 1: Get condition ID
    condition_id = get_market_condition_id(market_slug)
    if not condition_id:
        return traders

    # Step 2: Get holders from data-api
    try:
        url = f"https://data-api.polymarket.com/holders?market={condition_id}&limit=20"
        resp = requests.get(url, timeout=10)
        if resp.status_code != 200:
            return traders

        data = resp.json()
        if not data or not isinstance(data, list):
            return traders

        # Extract proxy wallets from holders
        for token_holders in data:
            holders = token_holders.get("holders", [])
            for holder in holders:
                wallet = holder.get("proxyWallet")
                if wallet and isinstance(wallet, str) and len(wallet) == 42:
                    traders.add(wallet.lower())

    except Exception as e:
        pass

    return traders


def fetch_activity(address: str, limit: int = 500) -> List[Dict]:
    """Fetch Polymarket activity for an address."""
    try:
        url = f"https://data-api.polymarket.com/activity?user={address}&limit={limit}"
        resp = requests.get(url, timeout=5)
        if resp.status_code == 200:
            return resp.json()
    except Exception:
        pass
    return []


def analyze_candidate(address: str, min_trades: int, min_win_rate: float,
                     max_avg_bet: float, min_participation: float) -> Dict | None:
    """
    Analyze a candidate address against bot criteria.

    Returns dict with metrics if candidate passes, None otherwise.
    """
    activity = fetch_activity(address)
    if not activity:
        return None

    # Filter for crypto updown markets
    markets = defaultdict(lambda: {
        "buy_usdc": 0.0,
        "sell_usdc": 0.0,
        "redeem_usdc": 0.0,
        "first_ts": None,
        "last_ts": None,
        "outcome": None
    })

    for event in activity:
        slug = event.get("eventSlug", "") or event.get("title", "")
        slug_lower = slug.lower()

        # Check if it's a crypto updown market
        if not ("updown" in slug_lower and ("5m" in slug_lower or "15m" in slug_lower)):
            continue

        if not any(c in slug_lower for c in CRYPTOS):
            continue

        type_ = event.get("type")
        usdc = float(event.get("usdcSize", 0))
        ts = event.get("timestamp")

        if type_ == "TRADE":
            side = event.get("side")
            outcome = event.get("outcome", "")

            if side == "BUY":
                markets[slug]["buy_usdc"] += usdc
                markets[slug]["outcome"] = outcome
                if ts and markets[slug]["first_ts"] is None:
                    markets[slug]["first_ts"] = ts
            elif side == "SELL":
                markets[slug]["sell_usdc"] += usdc

        elif type_ == "REDEEM":
            markets[slug]["redeem_usdc"] += usdc

        if ts and (markets[slug]["last_ts"] is None or ts > markets[slug]["last_ts"]):
            markets[slug]["last_ts"] = ts

    # Filter to markets with actual buys
    active_markets = {k: v for k, v in markets.items() if v["buy_usdc"] > 0}

    if not active_markets:
        return None

    total_markets = len(active_markets)

    if total_markets < min_trades:
        return None

    # Calculate win rate
    wins = 0
    losses = 0
    total_in = 0.0
    total_out = 0.0

    for m in active_markets.values():
        if m["buy_usdc"] > 0:
            pnl = (m["sell_usdc"] + m["redeem_usdc"]) - m["buy_usdc"]
            if pnl > 0.02:
                wins += 1
            elif pnl < -0.02:
                losses += 1
            total_in += m["buy_usdc"]
            total_out += m["sell_usdc"] + m["redeem_usdc"]

    total_decided = wins + losses
    if total_decided < 5:
        return None

    win_rate = (wins / total_decided * 100) if total_decided > 0 else 0
    avg_bet = total_in / total_markets

    # Calculate participation rate
    timestamps = sorted([
        m["first_ts"] for m in active_markets.values()
        if m["first_ts"] is not None
    ])

    participation_rate = 0.0
    if len(timestamps) >= 2:
        span = timestamps[-1] - timestamps[0]
        possible_windows = (span / 300) + 1  # 5-minute windows
        participation_rate = (total_markets / possible_windows * 100) if possible_windows > 0 else 0

    # Apply filters
    if win_rate < min_win_rate:
        return None
    if avg_bet > max_avg_bet:
        return None
    if participation_rate < min_participation:
        return None

    net_pnl = total_out - total_in

    return {
        "address": address,
        "total_markets": total_markets,
        "win_rate": win_rate,
        "wins": wins,
        "losses": losses,
        "avg_bet": avg_bet,
        "total_in": total_in,
        "total_out": total_out,
        "net_pnl": net_pnl,
        "participation_rate": participation_rate,
        "first_trade": datetime.fromtimestamp(timestamps[0]).isoformat() if timestamps else None,
        "last_trade": datetime.fromtimestamp(timestamps[-1]).isoformat() if timestamps else None,
    }


def main():
    parser = argparse.ArgumentParser(
        description="Market-First Bot Scanner for Polymarket Crypto UpDown",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python market_scanner.py --hours 12
  python market_scanner.py --hours 24 --min-wr 55 --max-avg-bet 100
  python market_scanner.py --hours 6 --cryptos btc eth --intervals 5m
        """
    )

    parser.add_argument(
        "--hours",
        type=int,
        default=24,
        help="Hours back to scan (default: 24)"
    )
    parser.add_argument(
        "--min-trades",
        type=int,
        default=20,
        help="Minimum trades in target markets (default: 20)"
    )
    parser.add_argument(
        "--min-wr",
        type=float,
        default=52.0,
        help="Minimum win rate percentage (default: 52.0)"
    )
    parser.add_argument(
        "--max-avg-bet",
        type=float,
        default=100.0,
        help="Maximum average bet size (default: 100.0)"
    )
    parser.add_argument(
        "--min-part",
        type=float,
        default=10.0,
        help="Minimum participation rate percentage (default: 10.0)"
    )
    parser.add_argument(
        "--cryptos",
        nargs="+",
        default=["btc", "eth", "sol"],
        help="Cryptos to scan (default: btc eth sol)"
    )
    parser.add_argument(
        "--intervals",
        nargs="+",
        default=["5m", "15m"],
        help="Intervals to scan (default: 5m 15m)"
    )
    parser.add_argument(
        "--output",
        type=str,
        help="Save candidates to JSON file"
    )

    args = parser.parse_args()

    # Update globals
    global CRYPTOS
    CRYPTOS = args.cryptos

    print("=" * 60)
    print("  MARKET-FIRST BOT SCANNER")
    print("=" * 60)
    print(f"Time Window: Last {args.hours} hours")
    print(f"Markets: {args.cryptos} - {args.intervals}")
    print(f"Filters: WR≥{args.min_wr}% | AvgBet≤${args.max_avg_bet} | Part≥{args.min_part}%")
    print("=" * 60)

    # Generate market slugs
    print(f"\n[1/3] Generating market slugs...")
    slugs = generate_market_slugs(args.hours)

    # Filter by user-specified intervals
    interval_filter = set(args.intervals)
    slugs = [s for s in slugs if any(i in s for i in interval_filter)]

    print(f"  Generated {len(slugs)} market slugs")

    # Collect traders from all markets
    print(f"\n[2/3] Fetching traders from markets...")
    all_traders = set()

    for i, slug in enumerate(slugs):
        traders = get_market_traders(slug)
        all_traders.update(traders)

        if (i + 1) % 50 == 0:
            print(f"  Scanned {i+1}/{len(slugs)} markets | Found {len(all_traders)} unique traders")

    print(f"  Complete: Found {len(all_traders)} unique traders")

    if not all_traders:
        print("\n[ERROR] No traders found. Check API connectivity.")
        return

    # Analyze candidates
    print(f"\n[3/3] Analyzing candidates...")
    candidates = []

    for i, address in enumerate(all_traders):
        print(f"  [{i+1}/{len(all_traders)}] Analyzing {address}...", end="\r")

        result = analyze_candidate(
            address,
            args.min_trades,
            args.min_wr,
            args.max_avg_bet,
            args.min_part
        )

        if result:
            candidates.append(result)
            print(f"\n  [!] CANDIDATE: {address}")
            print(f"      Win Rate: {result['win_rate']:.1f}% | Markets: {result['total_markets']}")
            print(f"      Avg Bet: ${result['avg_bet']:.2f} | Participation: {result['participation_rate']:.1f}%")
            print(f"      Net PnL: ${result['net_pnl']:+.2f}")

    # Results summary
    print("\n" + "=" * 60)
    print("  SCAN COMPLETE")
    print("=" * 60)
    print(f"  Markets Scanned: {len(slugs)}")
    print(f"  Traders Found: {len(all_traders)}")
    print(f"  Candidates Passing Filters: {len(candidates)}")

    if args.output and candidates:
        with open(args.output, "w") as f:
            json.dump(candidates, f, indent=2)
        print(f"\n  Saved candidates to: {args.output}")

    if candidates:
        print("\n  Top Candidates:")
        sorted_candidates = sorted(candidates, key=lambda x: x["win_rate"], reverse=True)
        for c in sorted_candidates[:5]:
            print(f"    {c['address']}")
            print(f"      WR: {c['win_rate']:.1f}% | PnL: ${c['net_pnl']:+.2f} | Markets: {c['total_markets']}")


if __name__ == "__main__":
    main()
