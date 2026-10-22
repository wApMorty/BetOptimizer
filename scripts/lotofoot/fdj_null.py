"""Hypothèse nulle « aucun savoir-faire » : 10 tickets tirés selon la répartition du public dans chaque grille, issues tirées
selon le marché, concurrents selon le modèle gelé. Probabilité d'atteindre le gain réalisé de la stratégie (409 grilles LF7/LF8)."""
import sys, json, numpy as np
sys.path.insert(0, "."); import backtest_fdj as BF
grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
prm = json.load(open("../data/lf/fdj_params_7_8.json")); TH = 11.34; NS = 4000; K = 10
REAL = sum(r["won"] for r in json.load(open("../data/lf/bt_fdj_all_7_8.json")))
rng = np.random.default_rng(1); sims = np.zeros(NS); cost = 0
for g in grids:
    n = g["n"]; P = np.array(g["p"]); Q = np.array(g["q"]); M = BF.masses(g); R = len(M); S = g["S"]; cost += K
    T = (rng.random((NS, K, n))[..., None] > np.cumsum(Q, axis=1)[None, None, :, :-1]).sum(axis=3)   # tickets « public »
    O = (rng.random((NS, n))[..., None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
    a = Q[np.arange(n)[None, :], O]; dist = np.zeros((NS, n + 1)); dist[:, 0] = 1
    for i in range(n):
        x = a[:, i:i + 1]; dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    Gd = (T == O[:, None, :]).sum(axis=2)
    for r in range(R):
        mu = prm["kappa"] * np.power(np.maximum(S * dist[:, n - r], 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
        W = rng.poisson(rng.gamma(TH, mu / TH)); c = (Gd == n - r).sum(axis=1)
        sims += np.where(c > 0, M[r] * c / (W + c), 0.0)
print(f"{len(grids)} grilles, {cost} € : gain réalisé de la stratégie {REAL:.0f} € ; sous l'hypothèse nulle, gain moyen {sims.mean():.0f} € "
      f"(ROI {sims.mean()/cost-1:+.1%}), p95 {np.percentile(sims,95):.0f} €, P(gain nul ≥ réalisé) = {np.mean(sims >= REAL):.3f}")
