"""Étape A : frontière ex ante espérance / fréquence de gain, grille par grille, sans aucun résultat de match.
Pour chaque grille appariée (vrai q, p d'avant-match, S, garantie) et chaque μ, 10 tickets gloutons maximisant
EV(t) + μ · P(t payé), distincts d'au moins 2 signes. On garde, pour k = 1 à 10 : l'espérance totale et P(au moins un ticket
payé) (Monte Carlo sur les issues tirées selon p, 20 000 tirages)."""
import sys, json, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast
import backtest_fdj as BF
MUS = [0.0, 3.0, 10.0, 30.0]; K = 10
PRM = {7: json.load(open("../data/lf/fdj_params_7_8.json")), 8: json.load(open("../data/lf/fdj_params_7_8.json")),
       12: json.load(open("../data/lf/fdj_params_12_15.json")), 15: json.load(open("../data/lf/fdj_params_12_15.json"))}
def best_ticket(P, Q, S, M, prm, mu, picked, starts, dmin=2):
    n = P.shape[0]; cache = {}
    def J(t):
        k = tuple(t)
        if k not in cache:
            e, pw = lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"]); cache[k] = (e + mu * pw, e, pw)
        return cache[k][0]
    ok = lambda t: all(sum(a != b for a, b in zip(t, u)) >= dmin for u in picked)
    best = None
    for t0 in starts + picked[-1:]:
        t = list(t0)
        if not ok(t):
            for i in range(n):           # perturbe le départ jusqu'à le rendre admissible
                t2 = list(t); t2[i] = (t2[i] + 1) % 3
                if ok(t2): t = t2; break
            if not ok(t): continue
        cur = J(t)
        for _ in range(6):
            imp = False
            for i in range(n):
                for o in range(3):
                    if o == t[i]: continue
                    t2 = list(t); t2[i] = o
                    if not ok(t2): continue
                    v = J(t2)
                    if v > cur + 1e-12: t, cur, imp = t2, v, True
            if not imp: break
        if best is None or cur > best[0]: best = (cur, t)
    if best is None:   # aucun départ admissible : tirage au hasard d'un ticket admissible (graine fixe, reproductible)
        rng = np.random.default_rng(len(picked))
        while best is None:
            t = [int(x) for x in rng.integers(0, 3, n)]
            if ok(t): best = (J(t), t)
    return best[1], cache[tuple(best[1])][1], cache[tuple(best[1])][2]
def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]; n = g["n"]; R = len(M)
    starts = [[int(np.argmax(x)) for x in P], [int(np.argmax(P[i] / Q[i])) for i in range(n)]]
    rng = np.random.default_rng(g["gid"])
    O = (rng.random((20000, n))[..., None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
    out = {"gid": g["gid"], "game": g["game"], "close": g["close"], "S": S, "jackpot": (g.get("jackpot") or {}).get("amount", 0), "mu": {}}
    for mu in MUS:
        picked, evs, pws, anyp = [], [], [], []
        for k in range(K):
            t, e, pw = best_ticket(P, Q, S, M, prm, mu, picked, starts); picked.append(t); evs.append(e); pws.append(pw)
            G = (np.array(picked)[None, :, :] == O[:, None, :]).sum(axis=2)
            anyp.append(float(np.mean((G >= n - R + 1).any(axis=1))))
        out["mu"][str(mu)] = {"tickets": picked, "ev": evs, "pwin": pws, "p_any": anyp}
    return out
if __name__ == "__main__":
    import os
    grids = json.load(open("../data/lf/grids_fdj.json"))
    part = "../data/lf/frontier_partial.jsonl"
    done = {json.loads(l)["gid"] for l in open(part)} if os.path.exists(part) else set()
    todo = [g for g in grids if g["gid"] not in done]
    with Pool(8) as pool, open(part, "a") as f:
        for i, r in enumerate(pool.imap_unordered(work, todo, chunksize=1)):
            f.write(json.dumps(r) + "\n"); f.flush()
            if (i + 1) % 25 == 0: print(f"{len(done)+i+1}/{len(grids)}", flush=True)
    res = [json.loads(l) for l in open(part)]
    json.dump(res, open("../data/lf/frontier.json", "w"))
    print("fini", flush=True)
