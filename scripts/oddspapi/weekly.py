#!/usr/bin/env python3
"""Test prospectif hebdomadaire de la latence française (docs/sources.md §34). Lecture seule sur edge.db.

Chaque passage : liste les matchs terminés des 8 derniers jours (6 requêtes décomptées sur 250 par mois), télécharge leur
historique OddsPapi (gratuit), puis applique la règle figée à TOUS les matchs accumulés depuis le début du test.

RÈGLE FIGÉE LE 2026-10-22, AVANT TOUT MATCH DU TEST (coup d'envoi ≥ 2026-10-22T00:00Z) :
- celle de latency.py / study.py : 1X2, Winamax + Unibet, issue de plus grand avantage contre Pinnacle sans marge (puissance)
  au même instant, seuil 1 %, fenêtre 60–15 min, un pari par match et par opérateur, CLV contre le dernier prix Pinnacle ;
- filtre de qualité : l'opérateur doit avoir au moins 10 changements de prix relevés dans les 24 h avant le coup d'envoi
  (20 en lecture secondaire) ;
- verdict une seule fois, dès 32 paris (puissance de 80 % pour une CLV de 2 %, σ = 4,5 %) : succès si CLV ≥ +1 % et borne
  basse de l'intervalle à 95 % > 0 (bootstrap ligue × semaine). Moins de 32 paris au 2027-01-18 (12 semaines) : signal
  trop rare pour être utile, quel que soit son signe.
- contrôle de qualité : part des cotes Winamax et Unibet identiques à celles de notre archive The Odds API aux mêmes instants.
"""
import datetime as dt
import json
import os
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import study  # noqa: E402  (règles et bootstrap partagés avec l'étude rétrospective)

START = "2026-10-22T00:00:00"
LEAGUES = {17: "Premier League", 8: "LaLiga", 35: "Bundesliga", 23: "Serie A", 34: "Ligue 1", 36: "Premiership"}
BOOKS = "pinnacle,winamax.fr,unibet.fr"
MIN_CHANGES, MIN_BETS, DEADLINE = 10, 32, "2027-01-18"
DATA = os.path.join(os.environ.get("XDG_DATA_HOME", os.path.expanduser("~/.local/share")), "edge", "oddspapi-weekly")
KEY = open(os.path.join(os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config")), "edge", "oddspapi.key")).read().strip()
_last = {}


def call(endpoint, cooldown, **params):
    """Appel GET ; l'URL porte la clé, elle n'est donc jamais affichée."""
    wait = _last.get(endpoint, 0) + cooldown - time.time()
    if wait > 0:
        time.sleep(wait)
    q = urllib.parse.urlencode({**params, "apiKey": KEY})
    req = urllib.request.Request(f"https://api.oddspapi.io/v4/{endpoint}?{q}", headers={"User-Agent": "edge-study/0.1"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            body = e.read().decode(errors="replace")
            if e.code in (400, 404):
                return None
            # 429 de débit (pas de quota épuisé) : on attend et on réessaie ; le quota mensuel, lui, arrête tout.
            if e.code == 429 and "REQUEST_LIMIT" not in body:
                time.sleep(10 * (attempt + 1))
                continue
            raise RuntimeError(f"{endpoint} : HTTP {e.code}") from None
        finally:
            _last[endpoint] = time.time()
    raise RuntimeError(f"{endpoint} : limite de débit persistante")


def fetch_week():
    now = dt.datetime.now(dt.timezone.utc)
    frm = max(now - dt.timedelta(days=8), dt.datetime.fromisoformat(START + "+00:00"))
    fixtures = []
    for tid in LEAGUES:
        fixtures += call("fixtures", 1.1, tournamentId=tid, statusId=2, **{"from": frm.strftime("%Y-%m-%dT%H:%M:%SZ"),
                                                                          "to": now.strftime("%Y-%m-%dT%H:%M:%SZ")}) or []
    new = 0
    for f in fixtures:
        path = os.path.join(DATA, "hist", f"{f['fixtureId']}_A.json")
        if f["startTime"] < START or os.path.exists(path):
            continue
        body = call("historical-odds", 5.3, fixtureId=f["fixtureId"], bookmakers=BOOKS)
        ko = dt.datetime.fromisoformat(f["startTime"].replace(".000Z", "+00:00"))
        keep = {}
        for bk, d in ((body or {}).get("bookmakers") or {}).items():
            for mid in ("101", "1010", "104"):
                for oid, o in (((d.get("markets") or {}).get(mid) or {}).get("outcomes") or {}).items():
                    pts = [(p["createdAt"], p["price"], p["active"]) for p in o["players"].get("0", [])
                           if dt.datetime.fromisoformat(p["createdAt"]) < ko]
                    if pts:
                        keep.setdefault(bk, {}).setdefault(mid, {})[oid] = pts
        json.dump({"fixture": f, "books": BOOKS, "odds": keep, "missing": body is None}, open(path, "w"))
        new += 1
    return len(fixtures), new


def changes(data, x):
    ko = study.ts(x["ko"].replace(".000Z", "+00:00"))
    d = data[x["fid"]]["books"].get(x["bk"], {}).get("101", {})
    return sum(1 for s in d.values() for c, *_ in s if ko - dt.timedelta(hours=24) <= study.ts(c) < ko)


def quality(data):
    """Part des cotes OddsPapi identiques à notre archive The Odds API au même instant (appariement par heure de coup d'envoi)."""
    db = os.path.join(os.environ.get("XDG_DATA_HOME", os.path.expanduser("~/.local/share")), "edge", "edge.db")
    c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = c.execute("""select a.called_at, e.commence_time, e.home_team_raw, e.away_team_raw, q.bookmaker_key, q.outcome_raw, q.price
        from quote q join event_seen e on e.api_call_id = q.api_call_id and e.event_id = q.event_id join api_call a on a.id = q.api_call_id
        where q.market_key = 'h2h' and q.bookmaker_key in ('winamax_fr', 'unibet_fr') and e.commence_time >= ?""", (START + "Z",)).fetchall()
    by_ko = {}
    for fid, d in data.items():
        by_ko.setdefault(d["fixture"]["startTime"][:16], []).append(d)
    same = total = 0
    for called, ko, h, a, bk, out, price in rows:
        cands = by_ko.get(ko[:16], [])
        # Appariement prudent : on ne garde que les créneaux où un seul match commence (pas d'appariement approximatif de noms).
        if len(cands) != 1:
            continue
        oid = "101" if out == h else "103" if out == a else "102"
        s = cands[0]["books"].get(bk.replace("_fr", ".fr"), {}).get("101", {}).get(oid)
        x = study.price_at(s, study.ts(called.replace("Z", "+00:00"))) if s else None
        if x:
            total += 1
            same += abs(x[0] - price) < 1e-9
    return same, total


def main():
    os.makedirs(os.path.join(DATA, "hist"), exist_ok=True)
    listed, new = fetch_week()
    os.chdir(DATA)  # study.load lit hist/*.json dans le dossier courant
    data = study.load()
    rows = [x for x in study.rows_for(data, "101", ["winamax.fr", "unibet.fr"]) if x["ko"] >= START]
    lines = [f"Test prospectif OddsPapi — {dt.datetime.now():%Y-%m-%d %H:%M}",
             f"Matchs listés cette semaine : {listed}, nouveaux téléchargés : {new}, total accumulé : {len(data)}"]
    for minch in (MIN_CHANGES, 20):
        first = {}
        for x in sorted((x for x in rows if x["h"] in study.WINDOW and x["edge"] >= study.THRESHOLD and changes(data, x) >= minch),
                        key=lambda x: -x["h"]):
            first.setdefault((x["fid"], x["bk"]), x)
        sel = list(first.values())
        m, lo, hi = study.boot([(x["block"], x["clv"]) for x in sel])
        tag = "règle principale" if minch == MIN_CHANGES else "lecture secondaire"
        lines.append(f"Suivi ≥ {minch} changements/24 h ({tag}) : {len(sel)} paris, CLV {100 * m:+.2f} % [{100 * lo:+.2f} ; {100 * hi:+.2f}]")
        if minch == MIN_CHANGES:
            if len(sel) >= MIN_BETS:
                ok = m >= 0.01 and lo > 0
                lines.append(f"VERDICT ({MIN_BETS} paris atteints) : {'SUCCÈS' if ok else 'ÉCHEC'}")
            elif dt.date.today().isoformat() >= DEADLINE:
                lines.append(f"VERDICT : moins de {MIN_BETS} paris au {DEADLINE}, signal trop rare pour être utile")
            else:
                lines.append(f"Pas de verdict avant {MIN_BETS} paris (échéance {DEADLINE})")
            for x in sorted(sel, key=lambda x: x["ko"])[-10:]:
                lines.append(f"   {x['ko'][:16]} {x['home'][:16]} – {x['away'][:16]} {x['bk']} {x['outcome']} "
                             f"cote {x['odds']:.2f} avantage {100 * x['edge']:+.1f} % CLV {100 * x['clv']:+.1f} %")
    same, total = quality(data)
    lines.append(f"Qualité : {same}/{total} cotes identiques à notre archive The Odds API" + (f" ({100 * same / total:.0f} %)" if total else ""))
    report = "\n".join(lines)
    print(report)
    open(os.path.join(DATA, f"rapport-{dt.date.today().isoformat()}.txt"), "w").write(report + "\n")


if __name__ == "__main__":
    main()
