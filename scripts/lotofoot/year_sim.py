"""Distribution du résultat annuel (10 €/semaine) pour quelques options, par simulation : pour chaque semaine historique, tickets
alloués comme dans frontier_report ; gains simulés (issues selon p, concurrents binomiale négative du modèle gelé) ; une année =
52 semaines tirées au hasard parmi les semaines historiques. Scénario S3 : p rapproché de q de 10 %, concurrents × 1,4 si λ < 3."""
import json, sys, datetime as dt, collections, numpy as np
sys.path.insert(0, "."); import backtest_fdj as BF
from frontier import PRM
TH = {7: 11.34, 8: 11.34, 12: 9.13, 15: 9.13}; NS = 3000
F = json.load(open("../data/lf/frontier.json")); G = {(g["game"], g["gid"]): g for g in json.load(open("../data/lf/grids_fdj.json"))}
def sim_grid(g, tickets, rng, shrink, low_mult):
    n = g["n"]; Q = np.array(g["q"]); P = (1 - shrink) * np.array(g["p"]) + shrink * Q; P /= P.sum(axis=1, keepdims=True)
    S = g["S"]; M = BF.masses(g); R = len(M); prm = PRM[g["game"]]; T = np.array(tickets)
    O = (rng.random((NS, n))[..., None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
    a = Q[np.arange(n)[None, :], O]; dist = np.zeros((NS, n + 1)); dist[:, 0] = 1
    for i in range(n):
        x = a[:, i:i + 1]; dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    good = (T[None, :, :] == O[:, None, :]).sum(axis=2); gain = np.zeros(NS)
    for r in range(R):
        lam = S * dist[:, n - r]
        mu = prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
        mu = np.where(lam < 3, mu * low_mult, mu)
        W = rng.poisson(rng.gamma(TH[g["game"]], mu / TH[g["game"]])); c = (good == n - r).sum(axis=1)
        gain += np.where(c > 0, M[r] * c / (W + c), 0.0)
    return gain
def option(games, mu, shrink, low_mult, seed=1):
    rng = np.random.default_rng(seed)
    weeks = collections.defaultdict(list)
    for r in F:
        if r["game"] in games: weeks[dt.date.fromisoformat(r["close"][:10]).isocalendar()[:2]].append(r)
    W = []
    for wk, gs in weeks.items():
        alloc = {(g["game"], g["gid"]): 0 for g in gs}; byid = {(g["game"], g["gid"]): g for g in gs}
        for _ in range(10):
            cand = [(byid[i]["mu"][mu]["ev"][alloc[i]], i) for i in alloc if alloc[i] < 10]
            alloc[max(cand)[1]] += 1
        tot = np.zeros(NS)
        for i, k in alloc.items():
            if k: tot += sim_grid(G[i], byid[i]["mu"][mu]["tickets"][:k], rng, shrink, low_mult)
        W.append(tot)
    W = np.array(W)                                    # semaines × tirages
    years = np.array([W[rng.integers(0, len(W), 52), rng.integers(0, NS, 52)].sum() for _ in range(20000)]) - 520
    return years
if __name__ == "__main__":
  for label, games, mu in (("A : LF7 seul, μ=10", (7,), "10.0"), ("B : tous jeux, μ=30", (7, 8, 12, 15), "30.0"), ("C : LF7+LF8, μ=30", (7, 8), "30.0")):
    for scen, sh, lm in (("base", 0.0, 1.0), ("S3", 0.1, 1.4)):
        y = option(games, mu, sh, lm)
        print(f"{label:22s} {scen:4s} : résultat net annuel moyen {y.mean():+6.0f} €, médiane {np.median(y):+6.0f} €, P(année positive) {np.mean(y>0):5.1%}, "
              f"p10 {np.percentile(y,10):+5.0f} €, p90 {np.percentile(y,90):+6.0f} €, P(perte > 400 €) {np.mean(y<-400):5.1%}", flush=True)
