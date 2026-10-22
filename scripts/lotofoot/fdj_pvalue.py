"""Gain réalisé contre la loi du gain simulée par le modèle (issues tirées selon le marché, concurrents selon le modèle gelé
avec sa dispersion θ), grille par grille, sur l'entraînement et sur le test de LF7/LF8."""
import sys, json, math, numpy as np
sys.path.insert(0, "."); import lf_fast, lf_ev
import backtest_fdj as BF
import backtest_lf as B
SPLIT = "2024-01-01"; NS = 4000
grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
prm = json.load(open("../data/lf/fdj_params_7_8.json")); TH = 11.34
B.K = 10
B.ev_fast = lambda t, p, q, S, m: lf_fast.ev(t, np.asarray(p), np.asarray(q), S, m, kappa=prm["kappa"], beta=prm["beta"], phi=prm["phi"])
rng = np.random.default_rng(20261022)
for period in ("entr.", "test"):
    sub = [g for g in grids if (g["close"] >= SPLIT) == (period == "test")]
    sims = np.zeros(NS); real = 0.0; cost = 0; evsum = 0.0
    for g in sub:
        n = g["n"]; P = np.array(g["p"]); Q = np.array(g["q"]); M = BF.masses(g); R = len(M); S = g["S"]
        tk = B.choose(g["p"], g["q"], S, M); T = np.array([t for _, t in tk]); cost += len(tk); evsum += sum(e for e, _ in tk)
        # gain réalisé (même règle que le backtest)
        Wpub = {n - r["good"]: r["W"] for r in g["ranks"]}
        good = (T == np.array(g["o"])[None, :]).sum(axis=1)
        for r in range(R):
            c = int(np.sum(good == n - r))
            if c: real += M[r] * c / (Wpub.get(r, 0) + c)
        # gains simulés
        U = rng.random((NS, n)); O = (U[:, :, None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
        a = Q[np.arange(n)[None, :], O]; dist = np.zeros((NS, n + 1)); dist[:, 0] = 1
        for i in range(n):
            x = a[:, i:i + 1]; dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
        Gd = (T[None, :, :] == O[:, None, :]).sum(axis=2)
        for r in range(R):
            mu = prm["kappa"] * np.power(np.maximum(S * dist[:, n - r], 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
            W = rng.poisson(rng.gamma(TH, mu / TH))                     # binomiale négative (mélange gamma-Poisson)
            c = (Gd == n - r).sum(axis=1)
            sims += np.where(c > 0, M[r] * c / (W + c), 0.0)
    print(f"{period} : {len(sub)} grilles, {cost} € misés ; espérance annoncée {evsum/cost:.2f} ; simulée {sims.mean()/cost:.2f} "
          f"[p5 {np.percentile(sims,5)/cost:.2f} ; médiane {np.median(sims)/cost:.2f} ; p95 {np.percentile(sims,95)/cost:.2f}] ; "
          f"réalisé {real/cost:.3f} ; P(gain simulé ≤ réalisé) = {np.mean(sims <= real):.3f}")
