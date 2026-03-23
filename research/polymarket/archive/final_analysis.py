"""
Final count from positions data to determine true win/loss ratio
"""
import json

# Parse the positions data (I'll count from the raw data)
# From the positions API response, count all BTC 5m positions

# Lost positions (currentValue = 0, percentPnl = -100):
losses = [
    ("btc-updown-5m-1773390000", 30.00, "Down"),  # 4:20AM
    ("btc-updown-5m-1773394800", 308.55, "Down"),  # 5:40AM
    ("btc-updown-5m-1773383100", 129.72, "Up"),    # 2:25AM
    ("btc-updown-5m-1773390600", 208.97, "Down"),  # 4:30AM
    ("btc-updown-5m-1773381900", 349.86, "Down"),  # 2:05AM
    ("btc-updown-5m-1773381300", 211.37, "Down"),  # 1:55AM
    ("btc-updown-5m-1773394200", 337.41, "Down"),  # 5:30AM
    ("btc-updown-5m-1773400500", 191.88, "Down"),  # 7:15AM
    ("btc-updown-5m-1773393900", 218.53, "Down"),  # 5:25AM
    ("btc-updown-5m-1773392400", 173.46, "Down"),  # 5:00AM
    ("btc-updown-5m-1773396000", 169.09, "Down"),  # 6:00AM
    ("btc-updown-5m-1773357300", 296.67, "Down"),  # Mar12 7:15PM
    ("btc-updown-5m-1773393600", 33.11, "Up"),     # 5:20AM
    ("btc-updown-5m-1773418500", 29.99, "Up"),     # 12:15PM
    ("btc-updown-5m-1773392100", 235.10, "Up"),    # 4:55AM
    ("btc-updown-5m-1773396600", 89.70, "Down"),   # 6:10AM
    ("btc-updown-5m-1773391800", 287.13, "Up"),    # 4:50AM
    ("btc-updown-5m-1773385800", 129.21, "Up"),    # 3:10AM
    ("btc-updown-5m-1773427500", 29.98, "Up"),     # 2:45PM
    ("btc-updown-5m-1773448200", 217.39, "Up"),    # 8:30PM
    ("btc-updown-5m-1773359100", 90.80, "Up"),     # Mar12 7:45PM
    ("btc-updown-5m-1773416100", 15.00, "Up"),     # 11:35AM
    ("btc-updown-5m-1773379200", 197.02, "Up"),    # 1:20AM
    ("btc-updown-5m-1773425100", 167.87, "Up"),    # 2:05PM
    ("btc-updown-5m-1773385500", 153.19, "Down"),  # 3:05AM
    ("btc-updown-5m-1773414900", 128.51, "Down"),  # 11:15AM
    ("btc-updown-5m-1773389100", 157.60, "Up"),    # 4:05AM
    ("btc-updown-5m-1773446400", 29.96, "Up"),     # 8:00PM
    ("btc-updown-5m-1773388200", 101.64, "Up"),    # 3:50AM
    ("btc-updown-5m-1773382800", 98.58, "Down"),   # 2:20AM
    ("btc-updown-5m-1773379800", 98.56, "Down"),   # 1:30AM
    ("btc-updown-5m-1773363300", 88.86, "Down"),   # Mar12 8:55PM
    ("btc-updown-5m-1773400800", 49.67, "Up"),     # 7:20AM
    ("btc-updown-5m-1773428100", 98.50, "Up"),     # 2:55PM
    ("btc-updown-5m-1773399000", 78.83, "Up"),     # 6:50AM
    ("btc-updown-5m-1773417900", 78.75, "Down"),   # 12:05PM
    ("btc-updown-5m-1773426900", 98.82, "Down"),   # 2:35PM
    ("btc-updown-5m-1773414600", 64.22, "Down"),   # 11:10AM
    ("btc-updown-5m-1773422100", 34.13, "Up"),     # 1:15PM
    ("btc-updown-5m-1773395400", 31.65, "Down"),   # 5:50AM
    ("btc-updown-5m-1773419400", 29.71, "Down"),   # 12:30PM
    ("btc-updown-5m-1773397500", 29.82, "Down"),   # 6:25AM
]

# Winning positions (have realizedPnl > 0 or were redeemed with value):
# These are positions with tiny remaining shares but positive realizedPnl
wins = [
    ("btc-updown-5m-1773381000", 467.68, 203.81, "Up"),   # 1:50AM - sold most at profit
    ("btc-updown-5m-1773387300", 196.25, 73.53, "Up"),    # 3:35AM
    ("btc-updown-5m-1773388500", 404.99, 244.61, "Up"),   # 3:55AM
    ("btc-updown-5m-1773385200", 169.37, 114.72, "Up"),   # 3:00AM
    ("btc-updown-5m-1773381600", 194.73, 81.94, "Down"),  # 2:00AM
    ("btc-updown-5m-1773383400", 376.12, 134.88, "Up"),   # 2:30AM
    ("btc-updown-5m-1773384300", 443.04, 58.92, "Up"),    # 2:45AM
    ("btc-updown-5m-1773382200", 351.68, 135.56, "Down"), # 2:10AM
    ("btc-updown-5m-1773380400", 151.76, 48.26, "Up"),    # 1:40AM
    ("btc-updown-15m-1773397800", 161.62, 57.58, "Up"),   # 6:30AM 15m
]

# Also some markets where they traded BOTH sides (loss on one side shown):
# btc-updown-5m-1773387300 has BOTH Up (win) and Down (loss at -100%)
# btc-updown-5m-1773380400 has BOTH Up (win) and Down (loss at -100%)
# btc-updown-5m-1773392400 has Down (loss) but likely had Up wins in activity

total_loss_cost = sum(l[1] for l in losses)
total_win_cost = sum(w[1] for w in wins)
total_win_realized = sum(w[2] for w in wins)

print("=" * 60)
print("GUGEZH TRUE STRATEGY ANALYSIS")
print("=" * 60)
print(f"Total LOSING positions: {len(losses)}")
print(f"Total WINNING positions (with realized PnL): {len(wins)}")
print(f"Total capital lost on losing side: ${total_loss_cost:.2f}")
print(f"Total capital deployed on winning side: ${total_win_cost:.2f}")
print(f"Total realized PnL from wins: ${total_win_realized:.2f}")
print(f"NET from these positions: ${total_win_realized - total_loss_cost:.2f}")
print()

# The REAL strategy:
print("=" * 60)
print("THE REAL STRATEGY:")
print("=" * 60)
print("""
gugezh is NOT a directional dip buyer.

gugezh BETS ON BOTH SIDES OF EVERY MARKET.

For every 5m window, the bot:
1. Waits ~90s for price discovery
2. Buys the DIPPED side (the one that dropped to 0.30-0.40)
3. But ALSO hedges with SMALL lotto bets on the other side
4. The winning side gets sold at 0.92 before settlement
5. The losing side goes to $0

This is essentially a STRADDLE strategy:
- Buy both Yes and No tokens
- The winner pays 0.92-1.00
- The loser pays 0.00
- Profit = (winner payout) - (cost of both sides)

For this to work profitably, the TOTAL cost of both sides 
must be less than 1.00 (the guaranteed settlement value).

Example: Buy Yes @ 0.35 + Buy No @ 0.35 = 0.70 total
Winner pays 1.00, loser pays 0.00
Net profit = 1.00 - 0.70 = 0.30 per share (42% return)

But if prices are: Yes @ 0.52 + No @ 0.52 = 1.04 total
Winner pays 1.00, loser pays 0.00 
Net LOSS = 1.00 - 1.04 = -0.04 per share

KEY INSIGHT: The bot profits when it can buy BOTH sides 
for a combined total < 1.00 (i.e., the market is inefficient).
""")

# Count how many distinct market WINDOWS they played
all_slugs = set()
for l in losses:
    all_slugs.add(l[0])
for w in wins:
    all_slugs.add(w[0])
    
print(f"Total distinct market windows played: {len(all_slugs)}")

# Check which markets have BOTH a winning and losing position
loss_slugs = set(l[0] for l in losses)
win_slugs = set(w[0] for w in wins)
both = loss_slugs & win_slugs
print(f"Markets where BOTH sides were played: {len(both)}")
print(f"Markets with ONLY losing positions: {len(loss_slugs - win_slugs)}")
print(f"Markets with ONLY winning positions: {len(win_slugs - loss_slugs)}")
