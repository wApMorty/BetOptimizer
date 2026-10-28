#!/usr/bin/env python3
# Étude des cotes proches de la cote juste (docs/sources.md §60) : jouer une cote française à quelques dixièmes de point de
# la cote juste de Pinnacle (3,21 pour 3,22 ; 1,24 pour 1,23) est-il perdant ?
#
# PROTOCOLE, figé le 2026-10-28 avant toute lecture des résultats (choix de Paul : football d'octobre + NBA ; décision à 14 h,
# puis 60 min avant le coup d'envoi) :
# - Données OddsPapi. Football : Big 5 + Écosse, 1X2, coups d'envoi du 2026-10-01 au 2026-10-27 (téléchargés pour le §58,
#   ~/.local/share/edge/etudes/data/creneaux/ ; le §58 n'a lu que les avantages > 0 de la meilleure sélection, jamais la
#   tranche étudiée ici). NBA : vainqueur prolongations comprises (marché 111), matchs terminés du 2026-04-01 au 2026-10-27
#   (proche_download.py). Opérateurs : Winamax, Unibet, PMU (Betclic absent d'OddsPapi).
# - Cote valable à t : dernière entrée antérieure ou égale à t (aucune entrée postérieure lue) ; une suspension rend l'issue
#   indisponible ; un marché dont la somme des probabilités implicites sort de [1,00 ; 1,30] est ignoré (règle de la
#   production). Filtre de qualité : au moins 10 changements relevés chez l'opérateur dans les 24 h avant le match (§34).
# - Instants de décision : principal, le dernier 14 h 00 (heure de Paris) strictement avant le coup d'envoi (le relevé de la
#   production ; la veille pour la NBA) ; secondaire, 60 min avant le coup d'envoi.
# - Unité : chaque couple match × opérateur suivi × issue coté à t, TOUTES les issues (pas seulement la meilleure).
#   Avantage annoncé e = o · p_t − 1, p_t = Pinnacle sans marge à t (méthode puissance, celle de la production).
#   CLV = o · p_clôture − 1, clôture = dernière cote de Pinnacle avant le coup d'envoi, même méthode (Buchdahl).
#   Seules les unités avec e ∈ [−2 % ; +2 %) sont étudiées ; tranches [−2 ; −1), [−1 ; −0,5), [−0,5 ; 0), [0 ; 0,5),
#   [0,5 ; 1), [1 ; 2) %.
# - Mesures, par sport × instant : par tranche, nombre d'unités, avantage annoncé moyen, CLV moyenne avec intervalle à 95 %
#   (bootstrap par blocs championnat × semaine ISO, semaine seule en NBA, 2 000 tirages, graine 7), part des CLV > 0.
#   Régression des moindres carrés CLV = a + b · e sur toutes les unités de [−2 ; 2) %, intervalles par le même bootstrap :
#   a = CLV d'une cote annoncée exactement juste, b = part de l'avantage annoncé qui survit jusqu'à la clôture.
# - Lectures secondaires : régression par tranche de cote (< 1,5 ; [1,5 ; 3) ; ≥ 3) ; sensibilité à la méthode de retrait de
#   marge (multiplicative, Shin), appliquée à t et à la clôture. Le rapport compte les configurations essayées.
# - Critère : la tranche [0 ; 1 %) donne un avantage réel si sa CLV moyenne a une borne basse > 0. Le ROI réel n'est pas mesuré
#   (environ 14 000 paris pour distinguer +0,8 % de zéro). Étude descriptive : rien ne change en production sans décision.
# - Ajouté APRÈS la première lecture (2026-10-28), parce que la tranche étudiée s'est révélée presque vide : distribution de
#   l'avantage annoncé sur toutes les unités (sans filtre sur e), avec la CLV moyenne de l'ensemble. Purement descriptif.
#
# Usage : python3 -I scripts/oddspapi/proche.py > ~/.local/share/edge/etudes/data/proche/rapport.txt
# Progression sur stderr ; aucune écriture hors de la sortie standard.
import bisect
import datetime as dt
import glob
import json
import os
import random
import statistics
import sys
import time
from zoneinfo import ZoneInfo

ROOT = os.path.expanduser("~/.local/share/edge/etudes/data")
PARIS = ZoneInfo("Europe/Paris")
UTC = dt.timezone.utc
REF = "pinnacle"
BOOKS = ["winamax.fr", "unibet.fr", "pmu"]
SPORTS = {
    "football": {"dir": f"{ROOT}/creneaux/hist", "fixtures": f"{ROOT}/creneaux/fixtures.json",
                 "outcomes": ["101", "102", "103"]},
    "NBA": {"dir": f"{ROOT}/proche/hist_nba", "fixtures": f"{ROOT}/proche/fixtures_nba.json",
            "outcomes": ["111", "112"]},
}
INSTANTS = ["14 h", "60 min"]
PROD_HOUR = 14
MIN_CHANGES = 10
IMPLIED_RANGE = (1.0, 1.30)
BANDS = [(-0.02, -0.01), (-0.01, -0.005), (-0.005, 0.0), (0.0, 0.005), (0.005, 0.01), (0.01, 0.02)]
ODDS_RANGES = [(1.0, 1.5, "< 1,5"), (1.5, 3.0, "[1,5 ; 3)"), (3.0, 1e9, "≥ 3")]
RESAMPLES, SEED = 2000, 7
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


def load(sport):
    conf = SPORTS[sport]
    fixtures = json.load(open(conf["fixtures"]))
    outcomes = conf["outcomes"]
    matches = []
    for n, f in enumerate(fixtures, 1):
        books = {}
        for path in glob.glob(f"{conf['dir']}/{f['fixtureId']}_*.json"):
            for book, data in json.load(open(path))["odds"].items():
                books[book] = {o: Series(points) for o, points in data.items()}
        kickoff = ts(f["startTime"])
        if REF not in books or not all(o in books[REF] for o in outcomes):
            continue
        tracked = [b for b in BOOKS if b in books and all(o in books[b] for o in outcomes)
                   and sum(kickoff - 86400 <= t < kickoff for s in books[b].values() for t in s.times) >= MIN_CHANGES]
        iso = dt.datetime.fromtimestamp(kickoff, UTC).isocalendar()
        league = f["tournamentId"] if sport == "football" else "NBA"
        matches.append({"id": f["fixtureId"], "kickoff": kickoff, "books": books, "tracked": tracked,
                        "outcomes": outcomes, "block": (league, iso[0], iso[1])})
        if n % 50 == 0 or n == len(fixtures):
            log(f"Chargement {sport} {n} / {len(fixtures)}")
    return fixtures, matches


def decision_time(match, instant):
    if instant == "60 min":
        return match["kickoff"] - 3600
    day = dt.datetime.fromtimestamp(match["kickoff"], PARIS).date()
    t = dt.datetime(day.year, day.month, day.day, PROD_HOUR, tzinfo=PARIS).timestamp()
    if t >= match["kickoff"]:
        day -= dt.timedelta(days=1)
        t = dt.datetime(day.year, day.month, day.day, PROD_HOUR, tzinfo=PARIS).timestamp()
    return t


# ---------- Probabilités sans marge ----------

def odds_at(book, outcomes, t):
    odds = [book[o].at(t) for o in outcomes]
    if any(o is None for o in odds):
        return None
    implied = sum(1 / o for o in odds)
    return odds if IMPLIED_RANGE[0] <= implied <= IMPLIED_RANGE[1] else None


def multiplicative(odds):
    """Retrait multiplicatif : p_i = π_i / Σ π_j (π_i = 1 / o_i)."""
    pi = [1 / o for o in odds]
    return [x / sum(pi) for x in pi]


def power(odds):
    """Retrait par la puissance : p_i = π_i^k, k unique solution de Σ π_i^k = 1 (π_i = 1 / o_i).
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


def shin(odds):
    """Retrait de Shin (Shin 1993 ; forme de Štrumbelj 2014) : p_i = (√(z² + 4 (1 − z) π_i² / Σπ) − z) / (2 (1 − z)),
    z ∈ [0 ; 1) part des initiés, unique solution de Σ p_i = 1 (Σ p_i décroît avec z ; bissection)."""
    pi = [1 / o for o in odds]
    total = sum(pi)

    def probs(z):
        return [((z * z + 4 * (1 - z) * x * x / total) ** 0.5 - z) / (2 * (1 - z)) for x in pi]

    lo, hi = 0.0, 0.999
    for _ in range(100):
        z = (lo + hi) / 2
        if sum(probs(z)) > 1.0:
            lo = z
        else:
            hi = z
    return probs((lo + hi) / 2)


METHODS = {"puissance": power, "multiplicative": multiplicative, "Shin": shin}


# ---------- Unités ----------

def units(matches, instant, method, every=False):
    """Toutes les issues cotées par les opérateurs suivis à l'instant de décision, avec e ∈ [−2 % ; 2 %) (toutes si every)."""
    remove = METHODS[method]
    out, no_ref = [], 0
    for m in matches:
        t = decision_time(m, instant)
        ref = m["books"][REF]
        now, close = odds_at(ref, m["outcomes"], t), odds_at(ref, m["outcomes"], m["kickoff"] - 1)
        if now is None or close is None:
            no_ref += 1
            continue
        p_now, p_close = remove(now), remove(close)
        for b in m["tracked"]:
            odds = odds_at(m["books"][b], m["outcomes"], t)
            if odds is None:
                continue
            for i, o in enumerate(odds):
                edge = o * p_now[i] - 1
                if every or BANDS[0][0] <= edge < BANDS[-1][1]:
                    out.append({"book": b, "odds": o, "edge": edge, "clv": o * p_close[i] - 1, "block": m["block"]})
    return out, no_ref


# ---------- Statistiques ----------

def blocks_of(rows):
    by_block = {}
    for r in rows:
        by_block.setdefault(r["block"], []).append(r)
    return [by_block[k] for k in sorted(by_block, key=str)]


def bootstrap(rows, stat):
    """Statistique et intervalle à 95 % par bootstrap de blocs (percentiles 2,5 et 97,5) ; stat renvoie un tuple."""
    if not rows:
        return None
    blocks = blocks_of(rows)
    rng = random.Random(SEED)
    draws = []
    for _ in range(RESAMPLES):
        sample = [r for _ in blocks for r in rng.choice(blocks)]
        value = stat(sample)
        if value is not None:
            draws.append(value)
    point = stat(rows)
    if point is None or not draws:
        return None
    out = []
    for j in range(len(point)):
        col = sorted(d[j] for d in draws)
        out.append((point[j], col[int(0.025 * len(col))], col[int(0.975 * len(col)) - 1]))
    return out


def mean_clv(rows):
    return (statistics.fmean(r["clv"] for r in rows),) if rows else None


def ols(rows):
    """Moindres carrés CLV = a + b · e : b = cov(e, CLV) / var(e), a = moy(CLV) − b · moy(e)."""
    if len(rows) < 3:
        return None
    me = statistics.fmean(r["edge"] for r in rows)
    mc = statistics.fmean(r["clv"] for r in rows)
    var = sum((r["edge"] - me) ** 2 for r in rows)
    if var == 0:
        return None
    b = sum((r["edge"] - me) * (r["clv"] - mc) for r in rows) / var
    return (mc - b * me, b)


def pct(x, digits=2):
    return f"{100 * x:+.{digits}f} %"


def fmt(ci, as_pct=True):
    if ci is None:
        return "—"
    f = pct if as_pct else (lambda x: f"{x:+.2f}")
    return f"{f(ci[0])} [{f(ci[1])} ; {f(ci[2])}]"


# ---------- Rapport ----------

def band_table(rows):
    print(f"{'tranche de e':>16} {'unités':>7} {'e moyen':>9} {'CLV moyenne [IC 95 %]':>34} {'CLV > 0':>8}")
    for lo, hi in BANDS:
        sel = [r for r in rows if lo <= r["edge"] < hi]
        if not sel:
            print(f"{f'[{100 * lo:+.1f} ; {100 * hi:+.1f})':>16} {0:>7}")
            continue
        ci = bootstrap(sel, mean_clv)
        share = 100 * sum(r["clv"] > 0 for r in sel) / len(sel)
        print(f"{f'[{100 * lo:+.1f} ; {100 * hi:+.1f})':>16} {len(sel):>7} {pct(statistics.fmean(r['edge'] for r in sel)):>9} "
              f"{fmt(ci[0]) if ci else '—':>34} {share:>7.0f} %")


def regression_line(label, rows):
    ci = bootstrap(rows, ols)
    if ci is None:
        print(f"  {label:<22} {len(rows):>6} unités : —")
        return
    print(f"  {label:<22} {len(rows):>6} unités : a = {fmt(ci[0])}, b = {fmt(ci[1], as_pct=False)}")


def main():
    configurations = 0
    print("Étude des cotes proches de la cote juste (docs/sources.md §60) — protocole en tête de scripts/oddspapi/proche.py")
    for sport in SPORTS:
        log(f"Chargement {sport}")
        fixtures, matches = load(sport)
        print(f"\n######## {sport} ########")
        print(f"Matchs listés : {len(fixtures)} ; avec Pinnacle : {len(matches)} ; "
              f"couples match × opérateur suivis (≥ {MIN_CHANGES} changements en 24 h) : {sum(len(m['tracked']) for m in matches)}")
        for b in BOOKS:
            print(f"  {b} : suivi sur {sum(b in m['tracked'] for m in matches)} matchs, "
                  f"présent sur {sum(b in m['books'] for m in matches)}")
        for instant in INSTANTS:
            rows, no_ref = units(matches, instant, "puissance")
            print(f"\n== {sport}, décision à {instant}{' (dernier 14 h avant le coup d’envoi)' if instant == '14 h' else ' avant le coup d’envoi'}"
                  f", méthode puissance ==")
            print(f"Matchs sans cote Pinnacle valable à t ou à la clôture : {no_ref} ; unités avec e ∈ [−2 ; 2) % : {len(rows)} "
                  f"(" + ", ".join(f"{b.split('.')[0]} {sum(r['book'] == b for r in rows)}" for b in BOOKS) + ")")
            every, _ = units(matches, instant, "puissance", every=True)
            edges = sorted(r["edge"] for r in every)
            deciles = statistics.quantiles(edges, n=10)
            print(f"Toutes les unités (ajouté après lecture) : {len(every)} ; e médian {pct(statistics.median(edges), 1)}, "
                  f"p10 {pct(deciles[0], 1)}, p90 {pct(deciles[-1], 1)}, max {pct(edges[-1], 1)} ; e ≥ 0 : "
                  f"{sum(e >= 0 for e in edges)} ; e moyen {pct(statistics.fmean(edges))}, "
                  f"CLV moyenne {pct(statistics.fmean(r['clv'] for r in every))}")
            band_table(rows)
            print("Régression CLV = a + b · e (a : CLV d'une cote annoncée juste ; b : part de l'avantage qui survit) :")
            regression_line("toutes cotes", rows)
            configurations += 1
            for lo, hi, label in ODDS_RANGES:
                regression_line(f"cote {label}", [r for r in rows if lo <= r["odds"] < hi])
                configurations += 1
            print("Sensibilité à la méthode de retrait de marge (à t et à la clôture) :")
            for method in ("multiplicative", "Shin"):
                alt, _ = units(matches, instant, method)
                regression_line(method, alt)
                sel = [r for r in alt if 0.0 <= r["edge"] < 0.01]
                ci = bootstrap(sel, mean_clv)
                print(f"  {'':<22} tranche [0 ; 1 %) : {len(sel)} unités, CLV {fmt(ci[0]) if ci else '—'}")
                configurations += 1
            log(f"{sport}, décision à {instant} : fait")
    print(f"\nConfigurations de régression essayées : {configurations} "
          f"(2 sports × 2 instants × (1 + 3 tranches de cote + 2 méthodes)), plus 6 tranches d'avantage par sport × instant.")
    log("Terminé")


if __name__ == "__main__":
    main()
