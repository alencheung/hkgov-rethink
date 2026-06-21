#!/usr/bin/env python3
"""Example: query the hkgov-rethink API for insights and records.

Usage:
  BASE_URL=http://localhost:8080 python3 examples/query_api.py
  BASE_URL=http://localhost:8080 API_KEY=secret python3 examples/query_api.py

Requires Python 3.9+ with `requests` (pip install requests).
"""
import os
import json
import sys

try:
    import requests
except ImportError:
    sys.exit("install requests:  pip install requests")

BASE = os.environ.get("BASE_URL", "http://localhost:8080")
KEY = os.environ.get("API_KEY", "")
PREFIX = "/v1"
HEADERS = {"X-API-Key": KEY} if KEY else {}


def get(path):
    r = requests.get(f"{BASE}{path}", headers=HEADERS, timeout=10)
    r.raise_for_status()
    return r.json()


def main():
    print(f"=== {BASE}{PREFIX}/sources ===")
    for s in get(f"{PREFIX}/sources"):
        print(f"  {s['source']}/{s['dataset']}: {s['record_count']} records — {s['title']}")

    print(f"\n=== {PREFIX}/datasets/hkma/capital-market-statistics/records (latest 3) ===")
    page = get(f"{PREFIX}/datasets/hkma/capital-market-statistics/records?limit=3")
    print(f"  total: {page['total']}")
    for rec in page["records"]:
        hs = rec["fields"].get("eq_mkt_hs_index")
        print(f"  {rec['record_id']}: Hang Seng Index = {hs}")

    print(f"\n=== {PREFIX}/insights (latest 5) ===")
    insights = get(f"{PREFIX}/insights?limit=5")
    if not insights:
        print("  (no insights yet — set [agent] enabled=true in config.toml)")
    for i in insights:
        print(f"  [{i['severity']}] {i['title']}")
        print(f"      {i['summary']}")
        print(f"      producer={i['producer']} confidence={i['confidence']:.0%}")


if __name__ == "__main__":
    main()
