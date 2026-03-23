import csv
import locale
import sys

# Change default encoding for clean printing in Windows PowerShell
sys.stdout.reconfigure(encoding='utf-8')

csv_path = r"c:\Users\mehul\OneDrive\Desktop\Studio\PROJECTS\polycli\polycli\export-address-token-0xbaa4b61faae1e8f90122963b44220e34040feaa4.csv"
target_address = "0xbaa4b61faae1e8f90122963b44220e34040feaa4"
ctf_exchange = "0x4bfb41d5b3570defd03c39a9a4d8de6bd8b8982e" 
polymarket_proxy = "0x4d97dcd97ec945f40cf65f87097ace5ea0476045"
usdc_contract = "0x2791bca1f2de4661ed88a30c99a7a9449aa84174"

external_in = 0.0
external_out = 0.0
ctf_in = 0.0
ctf_out = 0.0
proxy_in = 0.0
proxy_out = 0.0

withdrawal_destinations = {}

with open(csv_path, newline='', encoding='utf-8') as f:
    reader = csv.DictReader(f)
    for row in reader:
        if row["ContractAddress"].lower() != usdc_contract:
            continue
            
        val = float(row["TokenValue"].replace(",", ""))
        sender = row["From"].lower()
        receiver = row["To"].lower()
        
        if receiver == target_address:
            # IN
            if sender == ctf_exchange:
                ctf_in += val
            elif sender == polymarket_proxy:
                proxy_in += val
            else:
                external_in += val
                
        elif sender == target_address:
            # OUT
            if receiver == ctf_exchange:
                ctf_out += val
            elif receiver == polymarket_proxy:
                proxy_out += val
            else:
                external_out += val
                if receiver not in withdrawal_destinations:
                    withdrawal_destinations[receiver] = 0
                withdrawal_destinations[receiver] += val

print("--- FUNDING SUMMARY (USDC.e) ---")
print(f"External Deposits IN     : ${external_in:,.2f}")
print(f"External Withdrawals OUT : ${external_out:,.2f}")
print(f"Net Funded (Ext IN - OUT): ${(external_in - external_out):,.2f}")

print("\nWhere did the withdrawals go?")
for dest, amt in withdrawal_destinations.items():
    print(f"  -> {dest}: ${amt:,.2f}")

print("\n--- POLYMARKET CONTRACTS SUMMARY ---")
print(f"Sent TO CTF Exchange      : ${ctf_out:,.2f}")
print(f"Rcvd FROM CTF Exchange    : ${ctf_in:,.2f}")
print(f"Sent TO PM Proxy          : ${proxy_out:,.2f}")
print(f"Rcvd FROM PM Proxy        : ${proxy_in:,.2f}")

total_pm_out = ctf_out + proxy_out
total_pm_in = ctf_in + proxy_in
print(f"\nNet PM PnL (Rcvd - Sent)  : ${(total_pm_in - total_pm_out):,.2f}")
print(f"Total Bankroll Captured   : ${(total_pm_in - total_pm_out) + (external_in - external_out):,.2f}")

