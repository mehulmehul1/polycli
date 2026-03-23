import requests
import json
from datetime import datetime

wallet = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"
usdc_contract = "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"

url = f"https://api.polygonscan.com/api?module=account&action=tokentx&contractaddress={usdc_contract}&address={wallet}&page=1&offset=10000&sort=asc"
headers = {
    "User-Agent": "Mozilla/5.0"
}

print("Fetching USDC transfers from Polygonscan...")
resp = requests.get(url, headers=headers)
data = resp.json()

if data.get('status') == '1' and data.get('message') == 'OK':
    txs = data['result']
    total_in = 0
    total_out = 0
    
    print(f"Total USDC Transfers found: {len(txs)}")
    
    deposit_withdrawals_in = 0
    deposit_withdrawals_out = 0
    
    # 0x4bFB1616c276190FCd265780aAE481643c5b4e9e is usually the Polymarket CTF Exchange
    
    print("\n--- Summary of Transfers ---")
    for tx in txs:
        val = float(tx['value']) / 1e6
        ts = int(tx['timeStamp'])
        dt = datetime.fromtimestamp(ts).strftime('%Y-%m-%d %H:%M:%S')
        from_address = tx['from'].lower()
        to_address = tx['to'].lower()
        
        if to_address == wallet.lower():
            total_in += val
            # Only print massive transfers (likely deposits, not tiny trade payouts)
            if val > 1000:
                print(f"[{dt}] DEPOSIT/WIN : +${val:,.2f} from {from_address}")
        else:
            total_out += val
            if val > 1000:
                print(f"[{dt}] WITHDRAW/BUY: -${val:,.2f} to {to_address}")
            
    print("-" * 40)
    print(f"Total USDC IN : ${total_in:,.2f}")
    print(f"Total USDC OUT: ${total_out:,.2f}")
    print(f"Net USDC retained or generated : ${total_in - total_out:,.2f}")
    
else:
    print("Error or no transactions found:")
    print(data)
