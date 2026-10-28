#!/usr/bin/env python3
# Étude des cotes proches de la cote juste (docs/sources.md §60) : téléchargement de l'historique OddsPapi de la NBA.
#
# Le football vient de l'étude des créneaux (§58, ~/.local/share/edge/etudes/data/creneaux/), déjà sur le disque.
# Liste des matchs NBA terminés du 2026-04-01 au 2026-10-28 : `fixtures?tournamentId=132` (1 requête décomptée, faite à la
# main le 2026-10-28 et écrite dans fixtures_nba.json). Historique de chaque match (`historical-odds`, non décompté, 5 s entre
# deux appels) en deux passes : A = Pinnacle, Winamax, Unibet ; B = PMU. Seul l'avant-match du marché vainqueur, prolongations
# comprises (111 ; issues 111 = participant 1, 112 = participant 2), est conservé. Un fichier déjà écrit n'est jamais redemandé.
#
# Usage : python3 -I scripts/oddspapi/proche_download.py 2> ~/.local/share/edge/etudes/data/proche/download.log &
# La clé n'est jamais journalisée.
import datetime as dt
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

KEY = open(os.path.expanduser("~/.config/edge/oddspapi.key")).read().strip()
OUT = os.path.expanduser("~/.local/share/edge/etudes/data/proche")
BASE = "https://api.oddspapi.io/v4/"
MARKET = "111"
PASSES = [("A", "pinnacle,winamax.fr,unibet.fr"), ("B", "pmu")]
HISTORY_COOLDOWN = 5.3  # « endpoint cooldown : 5000 ms », avec une marge
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


def keep_prematch(body, kickoff):
    """Garde l'avant-match du marché vainqueur : bookmaker → issue → [(instant, prix, actif)]."""
    kept = {}
    for book, data in (body.get("bookmakers") or {}).items():
        market = (data.get("markets") or {}).get(MARKET)
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
    os.makedirs(f"{OUT}/hist_nba", exist_ok=True)
    fixtures = json.load(open(f"{OUT}/fixtures_nba.json"))
    jobs = [(tag, books, f) for tag, books in PASSES for f in fixtures
            if not os.path.exists(f"{OUT}/hist_nba/{f['fixtureId']}_{tag}.json")]
    total = len(jobs)
    log(f"Historiques NBA à télécharger : {total} (deux passes × {len(fixtures)} matchs, moins ceux déjà écrits) ; "
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
                  open(f"{OUT}/hist_nba/{fixture['fixtureId']}_{tag}.json", "w"))
        if n % 20 == 0 or n == total:
            rate = (time.time() - begin) / n
            log(f"Historiques {n} / {total} (passe {tag}) ; sans historique {missing}, échecs {failures} ; "
                f"reste environ {(total - n) * rate / 60:.0f} min")
    log(f"Terminé : {total - failures} historiques écrits, {missing} sans historique, {failures} échecs")


if __name__ == "__main__":
    main()
