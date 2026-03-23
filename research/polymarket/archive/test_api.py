import requests
import sys

address = "0xd0d6053c3c37e727402d84c14069780d360993aa"
url1 = f"https://api.etherscan.io/v2/api?chainid=137&module=account&action=tokentx&address={address}"
r1 = requests.get(url1)
print(r1.json())
