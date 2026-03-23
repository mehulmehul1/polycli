import requests
import json

API_KEY = "5P1T5B2K9EY1H789JEPIU9F5D23GQAU6K7"
PROXY_CONTRACT = "0x4d97dcd97ec945f40cf65f87097ace5ea0476045"

def test_etherscan():
    url = f"https://api.etherscan.io/v2/api?chainid=137&module=account&action=txlist&address={PROXY_CONTRACT}&startblock=0&endblock=99999999&page=1&offset=100&sort=desc&apikey={API_KEY}"
    print(f"URL: {url}")
    r = requests.get(url)
    print(f"Status: {r.status_code}")
    print(f"Response JSON: {r.json()}")

test_etherscan()
