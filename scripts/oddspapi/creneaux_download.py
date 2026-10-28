#!/usr/bin/env python3
# Étude des créneaux de détection (docs/sources.md §58) : téléchargement de l'historique OddsPapi d'octobre 2026.
#
# Déroulé : consommation du compte (`account`, non décompté), garde de budget, liste des matchs terminés de chaque
# championnat (`fixtures`, 6 requêtes décomptées), puis historique de chaque match (`historical-odds`, non décompté,
# 5 s entre deux appels) en deux passes : A = Pinnacle, Winamax, Unibet ; B = PMU. Seul l'avant-match du 1X2 (marché 101)
# est conservé. Reprise possible : un fichier déjà écrit n'est jamais redemandé.
#
# Usage : python3 -I scripts/oddspapi/creneaux_download.py 2> ~/.local/share/edge/etudes/data/creneaux/download.log &
# Données : ~/.local/share/edge/etudes/data/creneaux/ (hors dépôt, hors scratchpad). La clé n'est jamais journalisée.
import datetime as dt
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

KEY = open(os.path.expanduser("~/.config/edge/oddspapi.key")).read().strip()
OUT = os.path.expanduser("~/.local/share/edge/etudes/data/creneaux")
BASE = "https://api.oddspapi.io/v4/"
LEAGUES = {17: "E0", 8: "SP1", 35: "D1", 23: "I1", 34: "F1", 36: "SC0"}  # mêmes identifiants que crates/core/src/ingest/oddspapi
PERIOD = ("2026-10-01T00:00:00Z", "2026-10-28T00:00:00Z")  # coups d'envoi étudiés
PASSES = [("A", "pinnacle,winamax.fr,unibet.fr"), ("B", "pmu")]
HISTORY_COOLDOWN = 5.3  # « endpoint cooldown : 5000 ms », avec une marge
OTHER_COOLDOWN = 1.1
CAP = 240  # garde : on laisse 10 requêtes de marge sous l'allocation gratuite de 250
START = time.time()


def log(message):
    elapsed = time.time() - START
    print(f"{dt.datetime.now():%H:%M:%S} [{elapsed / 60:5.1f} min] {message}", file=sys.stderr, flush=True)


def get(path, params):
    """GET avec la clé ajoutée en dernier ; renvoie (statut, corps). Toute erreur est expurgée de la clé."""
    query = urllib.parse.urlencode({**params, "apiKey": KEY})
    request = urllib.request.Request(BASE + path + "?" + query, headers={"User-Agent": "edge-study/0.1"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                return response.status, response.read().decode()
        except urllib.error.HTTPError as error:
            body = error.read().decode(errors="replace")
            if error.code == 429 and "REQUEST_LIMIT" not in body:
                log(f"  débit refusé (429), nouvelle tentative dans {10 * (attempt + 1)} s")
                time.sleep(10 * (attempt + 1))
                continue
            return error.code, body
        except Exception as error:  # réseau : on réessaie, sans jamais afficher l'URL (elle contient la clé)
            log(f"  {type(error).__name__}, nouvelle tentative dans {10 * (attempt + 1)} s")
            time.sleep(10 * (attempt + 1))
    return 0, "échec réseau répété"


def account_usage():
    status, body = get("account", {})
    if status != 200:
        sys.exit(f"account : HTTP {status} {body[:200]}")
    sub = next(s for s in json.loads(body)["subscriptions"] if s["is_active"])
    return sub["request_count"], sub["request_limit"]


def list_fixtures():
    path = f"{OUT}/fixtures.json"
    if os.path.exists(path):
        fixtures = json.load(open(path))
        log(f"Liste des matchs déjà téléchargée : {len(fixtures)} matchs")
        return fixtures
    used, limit = account_usage()
    log(f"OddsPapi : {used} requêtes décomptées sur {limit} ; ce passage en ajoute {len(LEAGUES)}")
    if used + len(LEAGUES) > CAP:
        sys.exit(f"garde de budget : {used} + {len(LEAGUES)} > {CAP}, rien n'est demandé")
    fixtures = []
    for n, tournament in enumerate(LEAGUES, 1):
        time.sleep(OTHER_COOLDOWN)
        status, body = get("fixtures", {"tournamentId": tournament, "statusId": 2, "from": PERIOD[0], "to": PERIOD[1]})
        if status == 404 and "FIXTURE_NOT_FOUND" in body:
            rows = []
        elif status != 200:
            sys.exit(f"fixtures {tournament} : HTTP {status} {body[:200]}")
        else:
            rows = [f for f in json.loads(body) if f["statusId"] == 2 and PERIOD[0] <= f["startTime"] < PERIOD[1]]
        log(f"Matchs {n} / {len(LEAGUES)} : {LEAGUES[tournament]} {len(rows)} matchs terminés")
        fixtures += [{k: f[k] for k in ("fixtureId", "tournamentId", "startTime", "participant1Name", "participant2Name")}
                     for f in rows]
    fixtures.sort(key=lambda f: f["startTime"])
    json.dump(fixtures, open(path, "w"))
    return fixtures


def keep_prematch(body, kickoff):
    """Garde l'avant-match du 1X2 : bookmaker → issue → [(instant, prix, actif)], comme ingest/oddspapi/format.rs."""
    kept = {}
    for book, data in (body.get("bookmakers") or {}).items():
        market = (data.get("markets") or {}).get("101")
        if not market:
            continue
        for outcome, entries in market["outcomes"].items():
            points = [(e["createdAt"], e.get("price"), e.get("active", True))
                      for e in entries["players"].get("0", [])
                      if dt.datetime.fromisoformat(e["createdAt"]) < kickoff]
            if points:
                kept.setdefault(book, {})[outcome] = points
    return kept


def main():
    os.makedirs(f"{OUT}/hist", exist_ok=True)
    fixtures = list_fixtures()
    jobs = [(tag, books, f) for tag, books in PASSES for f in fixtures
            if not os.path.exists(f"{OUT}/hist/{f['fixtureId']}_{tag}.json")]
    total = len(jobs)
    log(f"Historiques à télécharger : {total} (deux passes × {len(fixtures)} matchs, moins ceux déjà écrits) ; "
        f"durée prévue {total * HISTORY_COOLDOWN / 60:.0f} min")
    begin, missing, failures = time.time(), 0, 0
    for n, (tag, books, fixture) in enumerate(jobs, 1):
        time.sleep(HISTORY_COOLDOWN)
        status, body = get("historical-odds", {"fixtureId": fixture["fixtureId"], "bookmakers": books})
        kickoff = dt.datetime.fromisoformat(fixture["startTime"].replace("Z", "+00:00"))
        if status == 200:
            odds = keep_prematch(json.loads(body), kickoff)
        elif status in (400, 404):
            odds, missing = {}, missing + 1
        else:
            failures += 1
            log(f"  {fixture['fixtureId']} passe {tag} : HTTP {status} {body[:150]} (redemandé au prochain lancement)")
            continue
        json.dump({"fixture": fixture, "books": books, "status": status, "odds": odds},
                  open(f"{OUT}/hist/{fixture['fixtureId']}_{tag}.json", "w"))
        if n % 20 == 0 or n == total:
            rate = (time.time() - begin) / n
            log(f"Historiques {n} / {total} (passe {tag}) ; sans historique {missing}, échecs {failures} ; "
                f"reste environ {(total - n) * rate / 60:.0f} min")
    used, limit = account_usage()
    log(f"Terminé : {total - failures} historiques écrits, {missing} sans historique, {failures} échecs ; "
        f"OddsPapi {used} / {limit} requêtes décomptées")


if __name__ == "__main__":
    main()
