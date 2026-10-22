"""LF7 + LF8 : résultat cumulé sur 1, 3, 5 et 10 ans (10 €/semaine) pour μ = 0, 10, 30, et l'option A (LF7 seul, μ = 10) en
référence ; modèle de base et scénario pessimiste S3. Part des tickets placés sur le LF8."""
import sys, json, datetime as dt, collections, numpy as np
sys.path.insert(0, "."); import year_sim as Y
def weekly(games, mu, shrink, low_mult, seed=1):
    rng = np.random.default_rng(seed); weeks = collections.defaultdict(list)
    for r in Y.F:
        if r["game"] in games: weeks[dt.date.fromisoformat(r["close"][:10]).isocalendar()[:2]].append(r)
    W, share8 = [], []
    for wk, gs in weeks.items():
        alloc = {(g["game"], g["gid"]): 0 for g in gs}; byid = {(g["game"], g["gid"]): g for g in gs}
        for _ in range(10):
            cand = [(byid[i]["mu"][mu]["ev"][alloc[i]], i) for i in alloc if alloc[i] < 10]
            alloc[max(cand)[1]] += 1
        share8.append(sum(k for i, k in alloc.items() if i[0] == 8) / 10)
        tot = np.zeros(Y.NS)
        for i, k in alloc.items():
            if k: tot += Y.sim_grid(Y.G[i], byid[i]["mu"][mu]["tickets"][:k], rng, shrink, low_mult)
        W.append(tot)
    return np.array(W), rng, float(np.mean(share8))
for label, games, mu in (("LF7+LF8, μ=0", (7, 8), "0.0"), ("LF7+LF8, μ=10", (7, 8), "10.0"), ("LF7+LF8, μ=30", (7, 8), "30.0"), ("A : LF7 seul, μ=10", (7,), "10.0")):
    for scen, sh, lm in (("base", 0.0, 1.0), ("S3", 0.1, 1.4)):
        W, rng, s8 = weekly(games, mu, sh, lm)
        cells = []
        for yrs in (1, 3, 5, 10):
            nw = 52 * yrs
            tot = np.array([W[rng.integers(0, len(W), nw), rng.integers(0, Y.NS, nw)].sum() for _ in range(8000)]) - 10 * nw
            cells.append(f"{yrs:>2} an(s) : médiane {np.median(tot):+6.0f} €, P {np.mean(tot>0):5.1%}")
        mean_y = (W.mean() - 10) * 52
        print(f"{label:20s} {scen:4s} (LF8 {s8:4.0%}, moyenne {mean_y:+5.0f} €/an) | " + " | ".join(cells), flush=True)
