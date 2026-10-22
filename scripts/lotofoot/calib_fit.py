"""Suite de calib.py : gagnants prédits λ_r contre gagnants observés W_r, régression de Poisson log E[W] = a + β log λ."""
import json, sys, math, numpy as np
sys.path.insert(0, "."); import lf_ev
from calib import qmodel
from scipy.optimize import minimize
def stakes(g):
    rk = g["ranks"]; n = len(g["matches"])
    if g["game"] == 15:
        ms = [r["winners"] * r["gain"] for r in rk[1:4] if r.get("winners")]
        if len(ms) < 2 or max(ms) > 1.1 * min(ms): return None
        return float(np.mean(ms)) / 0.14
    if g["game"] == 7:
        r2 = rk[1] if len(rk) > 1 else None
        if not r2 or not r2.get("winners"): return None
        return r2["winners"] * r2["gain"] / 0.385
def observations(grids):
    obs = []
    for g in grids:
        S = stakes(g)
        if S is None: continue
        n = len(g["ms"]); top = max(r["good"] for r in g["ranks"])
        if top != n: continue                       # rangs décalés (match annulé, ancienne formule) : écartés
        a = [qmodel(m["p"])[m["o"]] for m in g["ms"]]
        d = lf_ev.poisson_binomial(a)
        for r in g["ranks"]:
            lam = S * d[r["good"]]
            obs.append({"grid": f"{g['game']}-{g['date']}-{g['num']}", "game": g["game"], "rank": n - r["good"] + 1,
                        "lam": lam, "W": r["winners"] or 0, "S": S})
    return obs
def fit(obs):
    lam = np.array([o["lam"] for o in obs]); W = np.array([o["W"] for o in obs])
    nll = lambda x: float(np.sum(np.exp(x[0]) * lam ** x[1] - W * (x[0] + x[1] * np.log(lam))))
    return minimize(nll, [0.0, 1.0], method="Nelder-Mead").x
if __name__ == "__main__":
    grids = json.load(open(sys.argv[1])); obs = observations(grids)
    print(f"{len(obs)} observations sur {len({o['grid'] for o in obs})} grilles")
    for o in sorted(obs, key=lambda o: (o["grid"], o["rank"])):
        print(f"  {o['grid']:18s} rang {o['rank']}  λ {o['lam']:10.1f}  W {o['W']:6d}  ratio {(o['W']+0.5)/(o['lam']+0.5):6.2f}")
    a, b = fit(obs); print(f"log E[W] = {a:.3f} + {b:.3f} log λ   (κ = e^a = {math.exp(a):.2f})")
    rng = np.random.default_rng(20261022); ids = sorted({o["grid"] for o in obs}); B = []
    for _ in range(2000):
        pick = rng.choice(ids, len(ids)); sample = [o for i in pick for o in obs if o["grid"] == i]
        B.append(fit(sample))
    B = np.array(B); print(f"IC 95 % : a [{np.percentile(B[:,0],2.5):.3f} ; {np.percentile(B[:,0],97.5):.3f}], β [{np.percentile(B[:,1],2.5):.3f} ; {np.percentile(B[:,1],97.5):.3f}]")
    for lam0 in (0.1, 1, 10, 100, 1000):
        r = np.exp(B[:,0]) * lam0 ** B[:,1] / lam0
        print(f"  à λ = {lam0:6}: gagnants réels / prédits = {math.exp(a)*lam0**b/lam0:.2f} [{np.percentile(r,2.5):.2f} ; {np.percentile(r,97.5):.2f}]")
