"""Sonde : le marché `correct_score` existe-t-il pour le football dans The Odds API, chez quels bookmakers, avec quelle marge ? (1 crédit)"""
import json, os, sqlite3, tomllib, urllib.parse, urllib.request
db = sqlite3.connect("file:" + os.path.expanduser("~/.local/share/edge/edge.db") + "?mode=ro", uri=True)
ev = db.execute("""SELECT e.event_id, e.home_team_raw, e.away_team_raw, e.commence_time FROM event_seen e
                   WHERE e.sport_key='soccer_epl' AND e.commence_time > datetime('now') ORDER BY e.api_call_id DESC, e.commence_time LIMIT 1""").fetchone()
cfg = tomllib.load(open(os.path.expanduser("~/.config/edge/edge.toml"), "rb"))
key = os.environ.get("EDGE_ODDS_API_KEY") or cfg["odds_api_key"]
q = urllib.parse.urlencode({"apiKey": key, "regions": "eu", "markets": "correct_score", "oddsFormat": "decimal"})
url = f"https://api.the-odds-api.com/v4/sports/soccer_epl/events/{ev[0]}/odds?{q}"
try:
    with urllib.request.urlopen(url, timeout=30) as r:
        data = json.load(r); hdr = {k: r.headers.get(k) for k in ("x-requests-last", "x-requests-remaining")}
except urllib.error.HTTPError as e:
    print("HTTP", e.code, e.read().decode()[:300]); raise SystemExit
print(ev[1], "–", ev[2], ev[3], "| en-têtes", hdr)
for b in data.get("bookmakers", []):
    for m in b["markets"]:
        outs = m["outcomes"]
        marge = sum(1 / o["price"] for o in outs)
        sample = sorted(outs, key=lambda o: o["price"])[:4]
        print(f"  {b['key']:14s} {m['key']}: {len(outs)} scores, somme 1/cote = {marge:.3f} ; plus bas : " + ", ".join(f"{o['name']} {o.get('description','')} @{o['price']}" for o in sample))
if not data.get("bookmakers"): print("  aucun bookmaker ne propose ce marché pour ce match")
