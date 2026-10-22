# Téléchargement de l'historique OddsPapi (endpoint gratuit, 5 s entre deux appels) pour les matchs terminés des six ligues.
# On ne garde que l'avant-match (createdAt < coup d'envoi) des marchés 1X2 (101), plus/moins 2,5 (1010) et BTTS (104).
import json, os, time, datetime as dt, urllib.request, urllib.parse, urllib.error, sys
KEY = open(os.path.expanduser("~/.config/edge/oddspapi.key")).read().strip()
OUT = "hist"; os.makedirs(OUT, exist_ok=True)
fx = sorted(json.load(open("fixtures_finished.json")), key=lambda x: x["startTime"], reverse=True)
recent = "2026-08-01"
passes = [("A", "pinnacle,winamax.fr,unibet.fr", ""), ("C", "polymarket,kalshi", recent),
          ("D", "bet365.fr,bwin.fr,pokerstars.fr", recent), ("B", "pmu,zebet.fr,genybet.fr", "")]
last = 0.0; done = 0
for tag, books, since in passes:
    misses = 0  # 404 consécutifs : les matchs sont triés du plus récent au plus ancien, donc on a dépassé le début de l'historique
    for f in fx:
        if misses >= 15:
            print(f"passe {tag} : 15 matchs consécutifs sans historique, fin de la passe", flush=True); break
        if f["startTime"] < since: continue
        path = f"{OUT}/{f['fixtureId']}_{tag}.json"
        if os.path.exists(path): continue
        ko = f["startTime"].replace(".000Z", "+00:00")
        time.sleep(max(0, last + 5.3 - time.time()))
        q = urllib.parse.urlencode({"fixtureId": f["fixtureId"], "bookmakers": books, "apiKey": KEY})
        body = None
        for attempt in range(5):
            try:
                with urllib.request.urlopen(urllib.request.Request("https://api.oddspapi.io/v4/historical-odds?" + q, headers={"User-Agent": "edge-study/0.1"}), timeout=120) as r:
                    body = json.load(r); break
            except urllib.error.HTTPError as e:
                msg = e.read().decode(errors="replace")[:200]
                print(f"{f['fixtureId']} {tag} HTTP {e.code} {msg}", flush=True)
                if e.code == 429 and "REQUEST_LIMIT" in msg: sys.exit(1)
                if e.code in (400, 404):
                    json.dump({"fixture": f, "books": books, "odds": {}, "missing": True}, open(path, "w")); break
                time.sleep(10 * (attempt + 1))
            except Exception as e:
                print(f"{f['fixtureId']} {tag} {type(e).__name__}", flush=True); time.sleep(10 * (attempt + 1))
        last = time.time()
        if body is None:
            misses += os.path.exists(path); continue
        misses = 0
        keep = {}
        for bk, d in (body.get("bookmakers") or {}).items():
            for mid in ("101", "1010", "104"):
                m = (d.get("markets") or {}).get(mid)
                if not m: continue
                for oid, o in m["outcomes"].items():
                    pts = [p for p in o["players"].get("0", []) if dt.datetime.fromisoformat(p["createdAt"]) < dt.datetime.fromisoformat(ko)]
                    if pts: keep.setdefault(bk, {}).setdefault(mid, {})[oid] = [(p["createdAt"], p["price"], p["active"]) for p in pts]
        json.dump({"fixture": f, "books": books, "odds": keep}, open(path, "w"))
        done += 1
        if done % 50 == 0: print(time.strftime("%H:%M:%S"), tag, done, flush=True)
print("terminé", done, flush=True)
