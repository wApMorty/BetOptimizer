# Étude ponctuelle, lecture seule : valeur d'un freebet et conversion couverte chez les opérateurs ajoutés (OddsPapi).
# Instant de décision : chaque jour à 12:00 UTC (comme le relevé quotidien), matchs commençant dans les 7 jours.
# Qualité : un opérateur n'est utilisé sur un match que s'il a au moins 10 changements relevés dans les 24 h avant le match.
import json, glob, collections, statistics as st, datetime as dt
from study import ts, price_at, power
OUT = {"101": "H", "102": "D", "103": "A"}
data = collections.defaultdict(dict)
for p in glob.glob("hist/*.json"):
    d = json.load(open(p)); fid = d["fixture"]["fixtureId"]; data[fid]["fixture"] = d["fixture"]
    for bk, m in d["odds"].items():
        if "101" in m and len(m["101"]) == 3: data[fid][bk] = {o: sorted(v) for o, v in m["101"].items()}
def tracked(series, ko):
    return sum(1 for s in series.values() for c, *_ in s if ko - dt.timedelta(hours=24) <= ts(c) < ko) >= 10
BOOKS = ["winamax.fr", "unibet.fr", "pmu", "zebet.fr", "bet365.fr", "bwin.fr", "pokerstars.fr"]
CURRENT = ["winamax.fr", "unibet.fr", "pmu"]          # comptes de couverture déjà suivis (Betclic absent d'OddsPapi)
days = [dt.datetime(2026, 10, d, 12, tzinfo=dt.timezone.utc) for d in range(1, 19)]
rp = collections.defaultdict(list); over = collections.defaultdict(list)
best_g = collections.defaultdict(lambda: collections.defaultdict(list))
for t in days:
    day_best = collections.defaultdict(lambda: collections.defaultdict(float))
    for fid, d in data.items():
        f = d["fixture"]; ko = ts(f["startTime"].replace(".000Z", "+00:00"))
        if not (t < ko <= t + dt.timedelta(days=7)) or "pinnacle" not in d: continue
        pin = {o: price_at(d["pinnacle"][k], t) for k, o in OUT.items()}
        if any(v is None for v in pin.values()): continue
        p = dict(zip("HDA", power([pin[o][0] for o in "HDA"])))
        odds = {}
        for bk in BOOKS:
            if bk not in d or not tracked(d[bk], ko): continue
            q = {o: price_at(d[bk][k], t) for k, o in OUT.items()}
            if all(q.values()): odds[bk] = {o: q[o][0] for o in "HDA"}
        for bk, q in odds.items():
            over[bk].append(sum(1 / q[o] for o in "HDA"))
            for o in "HDA":
                rp[(bk, "3–4,5" if 3 <= q[o] < 4.5 else "4,5–6,5" if 4.5 <= q[o] < 6.5 else None)].append(q[o] * p[o] - p[o])
        # freebet couvert : pari bonus chez `bk` sur l'issue X, couverture des deux autres au meilleur prix des comptes.
        for bk in odds:
            for label, covers in (("actuels", CURRENT), ("tous", BOOKS)):
                pool = [c for c in covers if c != bk and c in odds]
                if not pool: continue
                for x in "HDA":
                    rest = [o for o in "HDA" if o != x]
                    g = (odds[bk][x] - 1) * (1 - sum(1 / max(odds[c][o] for c in pool) for o in rest))
                    day_best[(bk, label)]["g"] = max(day_best[(bk, label)]["g"], g)
    for k, v in day_best.items(): best_g[k[0]][k[1]].append(v["g"])
print("Surronde moyenne du 1X2 (octobre, relevé de 12 h, opérateur suivi) :")
for bk in BOOKS:
    if over[bk]: print(f"  {bk:14s} {st.mean(over[bk]):.3f}  (n = {len(over[bk])})")
print("\nValeur d'un freebet de 1 € sans couverture, r − p (moyenne) :")
for bk in BOOKS:
    row = [f"{band} : {st.mean(rp[(bk, band)]):.2f} (n={len(rp[(bk, band)])})" for band in ("3–4,5", "4,5–6,5") if rp[(bk, band)]]
    if row: print(f"  {bk:14s} " + " | ".join(row))
print("\nFreebet couvert, gain garanti par euro du meilleur match de chaque jour (médiane sur 18 jours ; min–max) :")
for bk in BOOKS:
    cells = []
    for label in ("actuels", "tous"):
        v = best_g[bk][label]
        if v: cells.append(f"couverture {label} : {st.median(v):.2f} ({min(v):.2f}–{max(v):.2f}, {len(v)} j)")
    if cells: print(f"  {bk:14s} " + " | ".join(cells))
