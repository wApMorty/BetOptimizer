"""Étape B : les tickets de la frontière, réévalués (sans réoptimisation) sous des hypothèses défavorables.
S1 : concurrents × 1,4 quand λ < 3 (sous-prédiction mesurée en LF12/LF15) ; S2 : p rapproché de q de 10 % (cotes du vendredi
qui surestiment les signes à contre-courant de 0,7 point) ; S3 : les deux. Écrit frontier_robust.json (même forme que frontier.json)."""
import json, sys, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast
import backtest_fdj as BF
from frontier import PRM
SCEN = {"S1": dict(low_mult=1.4, shrink=0.0), "S2": dict(low_mult=1.0, shrink=0.1), "S3": dict(low_mult=1.4, shrink=0.1)}
G = {(g["game"], g["gid"]): g for g in json.load(open("../data/lf/grids_fdj.json"))}
def work(r):
    g = G[(r["game"], r["gid"])]; Q = np.array(g["q"]); P0 = np.array(g["p"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    out = {k: {} for k in SCEN}
    for name, sc in SCEN.items():
        P = (1 - sc["shrink"]) * P0 + sc["shrink"] * Q; P = P / P.sum(axis=1, keepdims=True)
        for mu, v in r["mu"].items():
            out[name][mu] = [lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=sc["low_mult"])[0] for t in v["tickets"]]
    return (r["game"], r["gid"]), out
if __name__ == "__main__":
    F = json.load(open("../data/lf/frontier.json"))
    with Pool(8) as pool: res = dict(pool.map(work, F, chunksize=4))
    for name in SCEN:
        G2 = []
        for r in F:
            r2 = json.loads(json.dumps(r))
            for mu in r2["mu"]: r2["mu"][mu]["ev"] = res[(r["game"], r["gid"])][name][mu]
            G2.append(r2)
        json.dump(G2, open(f"../data/lf/frontier_{name}.json", "w"))
    print("fini")
