"""Calibration du modèle des concurrents avec la VRAIE répartition FDJ (protocole PROTOCOLE_FDJ.md).
λ_r = S · P_q(n − r + 1 bons | issue réelle) ; W_r publié. Modèle : W ~ Poisson(κ λ^β + plancher), ajusté par maximum de
vraisemblance (binomiale négative) sur l'entraînement (grilles closes avant 2024), puis contrôlé sur le test."""
import sys, glob, os, json, math, numpy as np
sys.path.insert(0, "."); import fdj_parse, lf_ev
from scipy.optimize import minimize
from scipy.special import gammaln
SPLIT = "2024-01-01"
def observations(grids):
    obs = []
    for g in grids:
        n = g["n"]; a = [g["q"][i][g["o"][i]] for i in range(n)]
        d = lf_ev.poisson_binomial(a)
        for r in g["ranks"]:
            if not (0 <= r["good"] <= n): continue
            obs.append({"gid": g["gid"], "game": g["game"], "close": g["close"], "rank": n - r["good"] + 1,
                        "lam": g["S"] * d[r["good"]], "u": g["S"] / 3 ** n, "W": r["W"]})
    return obs
def nll(x, lam, U, W):
    # Binomiale négative (variance μ + μ²/θ) : la dispersion est multiplicative d'une grille à l'autre ; une loi de Poisson
    # laisserait les rangs à des milliers de gagnants dicter l'ajustement (amendement du 2026-10-22, avant toute donnée de test).
    # Concurrents : κ λ^β + φ · S / 3^n (une part φ des mises répartie uniformément sur les 3^n combinaisons : joueurs à systèmes).
    k, b, fl, th = math.exp(x[0]), x[1], math.exp(x[2]), math.exp(x[3])
    mu = k * lam ** b + fl * U
    return float(-np.sum(gammaln(W + th) - gammaln(th) - gammaln(W + 1) + th * np.log(th / (th + mu)) + W * np.log(mu / (th + mu))))
def fit(obs):
    lam = np.array([o["lam"] for o in obs]); U = np.array([o["u"] for o in obs]); W = np.array([o["W"] for o in obs], dtype=float)
    r = minimize(nll, [0.0, 1.0, math.log(0.05), math.log(5.0)], args=(lam, U, W), method="Nelder-Mead", options={"maxiter": 8000, "xatol": 1e-7, "fatol": 1e-7})
    print(f"  dispersion θ = {math.exp(r.x[3]):.2f} (écart-type relatif d'une grille ≈ {1/math.sqrt(math.exp(r.x[3])):.2f})")
    return math.exp(r.x[0]), r.x[1], math.exp(r.x[2])
def table(obs, k, b, fl, label):
    print(label)
    for lo, hi in ((0, 0.3), (0.3, 1), (1, 3), (3, 10), (10, 100), (100, 1e3), (1e3, 1e4), (1e4, 1e9)):
        sub = [o for o in obs if lo <= o["lam"] < hi]
        if not sub: continue
        Wsum = sum(o["W"] for o in sub); lsum = sum(o["lam"] for o in sub); msum = sum(k * o["lam"] ** b + fl * o["u"] for o in sub)
        print(f"  λ ∈ [{lo:g} ; {hi:g}) : {len(sub):4d} obs, W observés {Wsum:9d}, λ brut {lsum:11.1f} (rapport {Wsum/max(lsum,1e-9):5.2f}), modèle {msum:11.1f} (rapport {Wsum/max(msum,1e-9):5.2f})")
if __name__ == "__main__":
    games = {int(x) for x in sys.argv[2].split(",")}; tag = "_".join(sorted(sys.argv[2].split(","), key=int))
    grids = [g for g in (fdj_parse.load(p)[0] for p in sorted(glob.glob(os.path.join(sys.argv[1], "*.json.gz")))) if g and g["game"] in games]
    obs = observations(grids)
    tr = [o for o in obs if o["close"] < SPLIT]; te = [o for o in obs if o["close"] >= SPLIT]
    print(f"{len(grids)} grilles valides ; observations : entraînement {len(tr)}, test {len(te)}")
    k, b, fl = fit(tr); print(f"entraînement : κ = {k:.3f}, β = {b:.4f}, φ = {fl:.4f}")
    json.dump({"kappa": k, "beta": b, "phi": fl}, open(f"../data/lf/fdj_params_{tag}.json", "w"))
    table(tr, k, b, fl, "ENTRAÎNEMENT")
    if len(sys.argv) > 3 and sys.argv[3] == "--test": table(te, k, b, fl, "TEST (paramètres gelés)")
