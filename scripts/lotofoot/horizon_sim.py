"""Résultat cumulé sur 1, 3, 5 et 10 ans (10 €/semaine) : option d'espérance maximale (tous jeux, μ = 0) contre option A (LF7, μ = 10),
modèle de base et scénario pessimiste S3. Mêmes tirages hebdomadaires que year_sim.py."""
import sys, json, datetime as dt, collections, numpy as np
sys.path.insert(0, "."); import year_sim as Y
def weekly(games, mu, shrink, low_mult, seed=1):
    rng = np.random.default_rng(seed); weeks = collections.defaultdict(list)
    for r in Y.F:
        if r["game"] in games: weeks[dt.date.fromisoformat(r["close"][:10]).isocalendar()[:2]].append(r)
    W = []
    for wk, gs in weeks.items():
        alloc = {(g["game"], g["gid"]): 0 for g in gs}; byid = {(g["game"], g["gid"]): g for g in gs}
        for _ in range(10):
            cand = [(byid[i]["mu"][mu]["ev"][alloc[i]], i) for i in alloc if alloc[i] < 10]
            alloc[max(cand)[1]] += 1
        tot = np.zeros(Y.NS)
        for i, k in alloc.items():
            if k: tot += Y.sim_grid(Y.G[i], byid[i]["mu"][mu]["tickets"][:k], rng, shrink, low_mult)
        W.append(tot)
    return np.array(W), rng
for label, games, mu in (("Espérance max : tous jeux, μ=0", (7, 8, 12, 15), "0.0"), ("A : LF7 seul, μ=10", (7,), "10.0")):
    for scen, sh, lm in (("base", 0.0, 1.0), ("S3", 0.1, 1.4)):
        W, rng = weekly(games, mu, sh, lm)
        cells = []
        for yrs in (1, 3, 5, 10):
            nw = 52 * yrs
            tot = np.array([W[rng.integers(0, len(W), nw), rng.integers(0, Y.NS, nw)].sum() for _ in range(8000)]) - 10 * nw
            cells.append(f"{yrs:>2} an(s) : médiane {np.median(tot):+6.0f} €, P(positif) {np.mean(tot>0):5.1%}")
        print(f"{label:32s} {scen:4s} | " + " | ".join(cells), flush=True)
