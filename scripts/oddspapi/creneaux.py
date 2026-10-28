#!/usr/bin/env python3
# Étude des créneaux de détection (docs/sources.md §58) : le passage de 14 h est-il le plus fructueux ?
#
# PROTOCOLE, figé le 2026-10-28 avant toute lecture des résultats (go de Paul, « tel quel ») :
# - Matchs : Big 5 + Écosse, coups d'envoi du 2026-10-01 au 2026-10-27 (OddsPapi, statut terminé), historique téléchargé
#   par creneaux_download.py. Opérateurs : Winamax, Unibet, PMU (Betclic absent d'OddsPapi). Marché 1X2.
# - Référence : dernière cote de Pinnacle connue à l'instant t, sans marge (méthode puissance, celle de la production).
#   Clôture : dernière cote de Pinnacle avant le coup d'envoi. CLV = o · p_clôture − 1 (Buchdahl, « Squares & Sharps »).
# - Cote valable à t : dernière entrée antérieure ou égale à t (aucune entrée postérieure lue : pas de fuite du futur) ;
#   une suspension rend l'issue indisponible. Une ligne dont la somme des probabilités implicites sort de [1,00 ; 1,30]
#   est ignorée (règle de la production).
# - Filtre de qualité : au moins 10 changements relevés chez l'opérateur dans les 24 h avant le match (§34).
# - Question principale : un passage quotidien unique à l'heure h (h = 0 … 23, heure de Paris, à h:00), règle de la
#   production : au plus une sélection par match (opérateur et issue de plus grand avantage), et seul le PREMIER passage
#   qui franchit le seuil compte (le journal garde la première ligne). Seuils : 1 % (recommandé) et > 0 % (tout avantage
#   positif). Mesures : nombre de paris, avantage annoncé, CLV moyenne avec intervalle à 95 % (bootstrap par blocs
#   championnat × semaine ISO, 2 000 tirages, graine 7), somme des CLV (gain attendu pour une mise de 1 par pari).
#   Comparaison : 14 h (le relevé de la production, 14 h 00, lu à 14 h 10). 48 configurations (24 heures × 2 seuils).
# - Critère : la CLV, jamais le nombre d'opportunités. Étude descriptive : rien ne change en production sans décision.
# - Lecture secondaire : délai avant le coup d'envoi (grille de 5 min sur les 72 dernières heures, premier franchissement
#   du seuil dans chaque tranche) ; durée de vie des écarts ≥ 1 % sur les 24 dernières heures (exacte, entre changements
#   de prix), par tranche de délai d'apparition.
# - Contrôle : relevés de 14 h (heure de Paris) de The Odds API contre l'état d'OddsPapi au même instant (Pinnacle et
#   opérateurs), et opportunités vues de part et d'autre.
#
# Usage : python3 -I scripts/oddspapi/creneaux.py > ~/.local/share/edge/etudes/data/creneaux/rapport.txt
# Progression sur stderr ; lecture seule sur la base (mode=ro).
import bisect
import datetime as dt
import glob
import json
import os
import random
import sqlite3
import statistics
import sys
import time
import tomllib
from zoneinfo import ZoneInfo

DATA = os.path.expanduser("~/.local/share/edge/etudes/data/creneaux")
DB = os.path.expanduser("~/.local/share/edge/edge.db")
ALIASES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../config/aliases.toml")
PARIS = ZoneInfo("Europe/Paris")
UTC = dt.timezone.utc
REF = "pinnacle"
BOOKS = ["winamax.fr", "unibet.fr", "pmu"]
OUTCOMES = ["101", "102", "103"]  # domicile, nul, extérieur
LABEL = {"101": "domicile", "102": "nul", "103": "extérieur"}
THRESHOLDS = [0.01, 0.0]
PROD_HOUR = 14
MIN_CHANGES = 10
IMPLIED_RANGE = (1.0, 1.30)
RESAMPLES, SEED = 2000, 7
LEAGUES = {17: ("E0", "soccer_epl"), 8: ("SP1", "soccer_spain_la_liga"), 35: ("D1", "soccer_germany_bundesliga"),
           23: ("I1", "soccer_italy_serie_a"), 34: ("F1", "soccer_france_ligue_one"), 36: ("SC0", "soccer_spl")}
DELAY_BINS = [(72 * 60, 24 * 60, "72–24 h"), (24 * 60, 12 * 60, "24–12 h"), (12 * 60, 6 * 60, "12–6 h"),
              (6 * 60, 3 * 60, "6–3 h"), (3 * 60, 60, "3–1 h"), (60, 30, "60–30 min"), (30, 15, "30–15 min"),
              (15, 0, "15–0 min")]
START = time.time()


def log(message):
    print(f"{dt.datetime.now():%H:%M:%S} [{time.time() - START:6.1f} s] {message}", file=sys.stderr, flush=True)


def ts(text):
    return dt.datetime.fromisoformat(text.replace("Z", "+00:00")).timestamp()


# ---------- Données ----------

class Series:
    """Série d'une issue chez un bookmaker : instants triés et cotes (None = suspendue ou sans prix)."""

    def __init__(self, points):
        points = sorted((ts(t), p if (active and p is not None and p > 1.0) else None) for t, p, active in points)
        self.times = [t for t, _ in points]
        self.prices = [p for _, p in points]

    def at(self, t):
        i = bisect.bisect_right(self.times, t) - 1
        return self.prices[i] if i >= 0 else None


def load_matches():
    fixtures = json.load(open(f"{DATA}/fixtures.json"))
    matches = []
    for n, f in enumerate(fixtures, 1):
        books = {}
        for tag in ("A", "B"):
            path = f"{DATA}/hist/{f['fixtureId']}_{tag}.json"
            if not os.path.exists(path):
                continue
            for book, outcomes in json.load(open(path))["odds"].items():
                books[book] = {o: Series(points) for o, points in outcomes.items()}
        kickoff = ts(f["startTime"])
        if REF not in books or not all(o in books[REF] for o in OUTCOMES):
            continue
        tracked = [b for b in BOOKS if b in books and all(o in books[b] for o in OUTCOMES)
                   and sum(kickoff - 86400 <= t < kickoff for s in books[b].values() for t in s.times) >= MIN_CHANGES]
        iso = dt.datetime.fromtimestamp(kickoff, UTC).isocalendar()
        matches.append({"id": f["fixtureId"], "league": LEAGUES[f["tournamentId"]][0], "kickoff": kickoff,
                        "home": f["participant1Name"], "away": f["participant2Name"], "books": books,
                        "tracked": tracked, "block": (f["tournamentId"], iso[0], iso[1])})
        if n % 50 == 0 or n == len(fixtures):
            log(f"Chargement {n} / {len(fixtures)}")
    return fixtures, matches


# ---------- Probabilités ----------

def odds_at(book, t):
    odds = [book[o].at(t) for o in OUTCOMES]
    if any(o is None for o in odds):
        return None
    implied = sum(1 / o for o in odds)
    return odds if IMPLIED_RANGE[0] <= implied <= IMPLIED_RANGE[1] else None


def power(odds):
    """Retrait de marge par la puissance : p_i = π_i^k, k unique solution de Σ π_i^k = 1 (π_i = 1 / o_i).
    Même méthode que crates/core/src/market/margin.rs (bissection sur k ∈ [0 ; 1000])."""
    pi = [1 / o for o in odds]
    lo, hi = 0.0, 1000.0
    for _ in range(100):
        k = (lo + hi) / 2
        if sum(x ** k for x in pi) > 1.0:
            lo = k
        else:
            hi = k
    k = (lo + hi) / 2
    return [x ** k for x in pi]


FAIR_CACHE = {}


def fair_at(match, t):
    """Probabilités de Pinnacle sans marge à t. Cache par instant du dernier changement (la valeur ne change qu'à un tick)."""
    ref = match["books"][REF]
    key = (match["id"], tuple(bisect.bisect_right(ref[o].times, t) for o in OUTCOMES))
    if key not in FAIR_CACHE:
        odds = odds_at(ref, t)
        FAIR_CACHE[key] = power(odds) if odds else None
    return FAIR_CACHE[key]


def closing(match):
    return fair_at(match, match["kickoff"] - 1)


def best_at(match, t, books=None):
    """Sélection de la production à t : opérateur et issue de plus grand avantage o · p − 1 (à égalité, le premier)."""
    fair = fair_at(match, t)
    if fair is None:
        return None
    best = None
    for b in books if books is not None else match["tracked"]:
        odds = odds_at(match["books"][b], t)
        if odds is None:
            continue
        for i, o in enumerate(OUTCOMES):
            edge = odds[i] * fair[i] - 1
            if best is None or edge > best["edge"]:
                best = {"book": b, "outcome": o, "odds": odds[i], "p": fair[i], "edge": edge}
    return best


def passes(edge, threshold):
    return edge >= threshold if threshold > 0 else edge > 0


def make_bet(match, t, sel, close):
    return {**sel, "match": match, "t": t, "clv": sel["odds"] * close[OUTCOMES.index(sel["outcome"])] - 1,
            "delay_h": (match["kickoff"] - t) / 3600, "block": match["block"]}


# ---------- Statistiques ----------

def mean_ci(bets):
    """Moyenne de la CLV et intervalle à 95 % par bootstrap de blocs championnat × semaine (percentiles 2,5 et 97,5)."""
    if not bets:
        return None
    by_block = {}
    for b in bets:
        by_block.setdefault(b["block"], []).append(b["clv"])
    blocks = sorted(by_block)
    rng = random.Random(SEED)
    means = []
    for _ in range(RESAMPLES):
        sample = [v for _ in blocks for v in by_block[rng.choice(blocks)]]
        means.append(sum(sample) / len(sample))
    means.sort()
    return (statistics.fmean(b["clv"] for b in bets), means[int(0.025 * RESAMPLES)], means[int(0.975 * RESAMPLES) - 1])


def pct(x):
    return f"{100 * x:+.1f} %"


def fmt_ci(ci):
    return "—" if ci is None else f"{pct(ci[0])} [{pct(ci[1])} ; {pct(ci[2])}]"


# ---------- Analyses ----------

def daily_scan(matches, hour, threshold):
    """Un passage par jour à hour:00 (Paris) ; premier passage qui franchit le seuil, comme le journal de la production."""
    bets = []
    for m in matches:
        close = closing(m)
        if close is None or not m["tracked"]:
            continue
        ko_day = dt.datetime.fromtimestamp(m["kickoff"], PARIS).date()
        for back in range(14, -1, -1):
            day = ko_day - dt.timedelta(days=back)
            t = dt.datetime(day.year, day.month, day.day, hour, tzinfo=PARIS).timestamp()
            if t >= m["kickoff"]:
                break
            sel = best_at(m, t)
            if sel and passes(sel["edge"], threshold):
                bets.append(make_bet(m, t, sel, close))
                break
    return bets


def delay_scan(matches, threshold):
    """Grille de 5 min sur les 72 dernières heures ; dans chaque tranche, premier instant qui franchit le seuil."""
    out = {label: [] for _, _, label in DELAY_BINS}
    for m in matches:
        close = closing(m)
        if close is None or not m["tracked"]:
            continue
        for start, end, label in DELAY_BINS:
            for minutes in range(start, end, -5):
                t = m["kickoff"] - 60 * minutes
                sel = best_at(m, t)
                if sel and passes(sel["edge"], threshold):
                    out[label].append(make_bet(m, t, sel, close))
                    break
    return out


def episodes(matches):
    """Écarts ≥ 1 % chez chaque opérateur suivi sur les 24 dernières heures : durée exacte (la cote et la référence sont
    constantes entre deux changements), délai d'apparition, CLV à l'apparition."""
    found = []
    for m in matches:
        close = closing(m)
        if close is None:
            continue
        lo = m["kickoff"] - 86400
        for b in m["tracked"]:
            times = sorted({t for book in (REF, b) for s in m["books"][book].values() for t in s.times
                            if lo < t < m["kickoff"]} | {lo})
            current = None
            for i, t in enumerate(times):
                sel = best_at(m, t, [b])
                on = sel is not None and sel["edge"] >= 0.01
                if on and current is None:
                    current = make_bet(m, t, sel, close)
                if not on and current is not None:
                    found.append((current, t - current["t"]))
                    current = None
            if current is not None:
                found.append((current, m["kickoff"] - current["t"]))
    return found


# ---------- Contrôle : The Odds API à 14 h contre OddsPapi ----------

def concordance(matches):
    aliases = tomllib.load(open(ALIASES, "rb"))
    canon = {}
    for league, table in aliases.items():
        for name, variants in table.items():
            for v in [name, *variants]:
                canon[(league, v)] = name
    index = {}
    unknown = set()
    for m in matches:
        h, a = canon.get((m["league"], m["home"])), canon.get((m["league"], m["away"]))
        if h is None or a is None:
            unknown.add((m["league"], m["home"] if h is None else m["away"]))
            continue
        index[(m["league"], h, a)] = m
    sport_to_league = {sport: code for code, sport in LEAGUES.values()}
    db = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    rows = db.execute("select called_at, sport_key, body from api_call where endpoint = 'odds' and http_status = 200 "
                      "and sport_key like 'soccer%'").fetchall()
    # Relevé de 14 h de Paris : 12:00 UTC à l'heure d'été (jusqu'au 25/10), 13:00 UTC ensuite.
    rows = [r for r in rows if dt.datetime.fromtimestamp(ts(r[0]), PARIS).hour == PROD_HOUR]
    ref_diff, opp_api, opp_papi, opp_both, pairs = [], 0, 0, 0, 0
    same, compared = {}, {}
    keys = {"winamax_fr": "winamax.fr", "unibet_fr": "unibet.fr", "pmu_fr": "pmu"}
    for called_at, sport, body in rows:
        league = sport_to_league.get(sport)
        if league is None:
            continue
        t = ts(called_at)
        events = json.loads(body)
        if not isinstance(events, list):  # relevé de clôture d'un seul match (un objet) : pas un relevé de 14 h
            continue
        for e in events:
            m = index.get((league, canon.get((league, e["home_team"])), canon.get((league, e["away_team"]))))
            if m is None or ts(e["commence_time"]) <= t:
                continue
            api = {}
            for bk in e["bookmakers"]:
                h2h = next((mk for mk in bk["markets"] if mk["key"] == "h2h"), None)
                if h2h is None:
                    continue
                price = {o["name"]: o["price"] for o in h2h["outcomes"]}
                odds = [price.get(e["home_team"]), price.get("Draw"), price.get(e["away_team"])]
                if None not in odds and IMPLIED_RANGE[0] <= sum(1 / o for o in odds) <= IMPLIED_RANGE[1]:
                    api[bk["key"]] = odds
            fair_papi = fair_at(m, t)
            if "pinnacle" not in api or fair_papi is None:
                continue
            pairs += 1
            fair_api = power(api["pinnacle"])
            ref_diff.append(max(abs(x - y) for x, y in zip(fair_api, fair_papi)))
            for key, book in keys.items():
                if key in api and book in m["books"]:
                    papi = odds_at(m["books"][book], t)
                    if papi:
                        compared[book] = compared.get(book, 0) + 3
                        same[book] = same.get(book, 0) + sum(abs(x - y) < 1e-9 for x, y in zip(api[key], papi))
            books = [k for k in keys if k in api]
            e_api = max((api[k][i] * fair_api[i] - 1 for k in books for i in range(3)), default=None)
            sel = best_at(m, t, [b for b in BOOKS if b in m["books"]])
            a_on = e_api is not None and e_api >= 0.01
            p_on = sel is not None and sel["edge"] >= 0.01
            opp_api += a_on
            opp_papi += p_on
            opp_both += a_on and p_on
    return {"pairs": pairs, "ref_diff": ref_diff, "same": same, "compared": compared, "opp_api": opp_api,
            "opp_papi": opp_papi, "opp_both": opp_both, "unknown": sorted(unknown)}


# ---------- Rapport ----------

def main():
    log("Chargement des historiques")
    fixtures, matches = load_matches()
    tracked_pairs = sum(len(m["tracked"]) for m in matches)
    print("Étude des créneaux de détection (docs/sources.md §58) — protocole en tête de scripts/oddspapi/creneaux.py")
    print(f"Matchs listés : {len(fixtures)} ; avec Pinnacle complet : {len(matches)} ; "
          f"couples match × opérateur suivis (≥ {MIN_CHANGES} changements en 24 h) : {tracked_pairs}")
    for b in BOOKS:
        print(f"  {b} : suivi sur {sum(b in m['tracked'] for m in matches)} matchs, "
              f"présent sur {sum(b in m['books'] for m in matches)}")

    for threshold in THRESHOLDS:
        name = "≥ 1 %" if threshold else "> 0 %"
        print(f"\n== Passage quotidien unique à l'heure h, seuil {name} ==")
        print(f"{'h':>3} {'paris':>6} {'avantage':>9} {'CLV moyenne [IC 95 %]':>32} {'Σ CLV':>7} {'délai méd.':>10}  opérateurs")
        for hour in range(24):
            bets = daily_scan(matches, hour, threshold)
            ci = mean_ci(bets)
            books = {b: sum(x["book"] == b for x in bets) for b in BOOKS}
            delay = f"{statistics.median(x['delay_h'] for x in bets):.0f} h" if bets else "—"
            mark = " ← production" if hour == PROD_HOUR else ""
            edge = pct(statistics.fmean(x["edge"] for x in bets)) if bets else "—"
            print(f"{hour:>3} {len(bets):>6} {edge:>9} {fmt_ci(ci):>32} {sum(x['clv'] for x in bets):>+7.2f} "
                  f"{delay:>10}  " + " ".join(f"{b.split('.')[0]} {n}" for b, n in books.items()) + mark)
            log(f"Passage quotidien, seuil {name} : heure {hour + 1} / 24")

    for threshold in THRESHOLDS:
        name = "≥ 1 %" if threshold else "> 0 %"
        print(f"\n== Délai avant le coup d'envoi (grille de 5 min), seuil {name} : premier franchissement par tranche ==")
        print(f"{'tranche':>10} {'matchs':>7} {'avantage':>9} {'CLV moyenne [IC 95 %]':>32} {'CLV > 0':>8}")
        for label, bets in delay_scan(matches, threshold).items():
            edge = pct(statistics.fmean(x["edge"] for x in bets)) if bets else "—"
            share = f"{100 * sum(x['clv'] > 0 for x in bets) / len(bets):.0f} %" if bets else "—"
            print(f"{label:>10} {len(bets):>7} {edge:>9} {fmt_ci(mean_ci(bets)):>32} {share:>8}")
        log(f"Délai avant le coup d'envoi, seuil {name} : fait")

    print("\n== Durée de vie des écarts ≥ 1 % (24 dernières heures, par opérateur suivi) ==")
    found = episodes(matches)
    log(f"Épisodes : {len(found)}")
    print(f"{'apparition':>10} {'écarts':>7} {'durée p25':>10} {'médiane':>8} {'p75':>7} {'CLV à l’apparition [IC 95 %]':>36}")
    for start, end, label in DELAY_BINS[1:]:
        rows = [(b, d) for b, d in found if end * 60 <= b["match"]["kickoff"] - b["t"] < start * 60]
        if not rows:
            print(f"{label:>10} {0:>7}")
            continue
        q = statistics.quantiles([d / 60 for _, d in rows], n=4, method="inclusive") if len(rows) > 1 else [rows[0][1] / 60] * 3
        print(f"{label:>10} {len(rows):>7} {q[0]:>8.1f} m {q[1]:>6.1f} m {q[2]:>5.1f} m "
              f"{fmt_ci(mean_ci([b for b, _ in rows])):>36}")
    for b in BOOKS:
        rows = [d / 60 for x, d in found if x["book"] == b]
        if rows:
            print(f"  {b} : {len(rows)} écarts, durée médiane {statistics.median(rows):.1f} min")

    log("Contrôle de concordance avec The Odds API")
    c = concordance(matches)
    print("\n== Contrôle : relevés de 14 h de The Odds API contre OddsPapi au même instant ==")
    print(f"Couples relevé × match rapprochés : {c['pairs']}")
    if c["ref_diff"]:
        print(f"Pinnacle sans marge, écart maximal entre les deux sources par match : médiane "
              f"{100 * statistics.median(c['ref_diff']):.2f} pt, 90e centile "
              f"{100 * statistics.quantiles(c['ref_diff'], n=10)[-1]:.2f} pt")
    for book, n in sorted(c["compared"].items()):
        print(f"Cotes identiques, {book} : {c['same'][book]} / {n} ({100 * c['same'][book] / n:.0f} %)")
    print(f"Opportunités ≥ 1 % (Winamax, Unibet, PMU) : The Odds API {c['opp_api']}, OddsPapi {c['opp_papi']}, "
          f"les deux {c['opp_both']}")
    if c["unknown"]:
        print(f"Noms OddsPapi absents de la table d'alias (matchs non rapprochés) : {c['unknown']}")
    log("Terminé")


if __name__ == "__main__":
    main()
