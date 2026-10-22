"""Règle « budget maximal » le samedi : sur les grilles LF7/LF8 ouvertes le samedi à 15 h, 16 tickets simples libres (μ = 10)
au plus ; un ticket n'est joué que si son espérance pessimiste (S3) dépasse τ. Dépense moyenne, gain annuel (base et S3)."""
import sys, json, gzip, glob, datetime as dt, collections, numpy as np
from zoneinfo import ZoneInfo
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM
PAR = ZoneInfo("Europe/Paris")
def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    T = B16.greedy(P, Q, S, M, prm, 16, 1)
    Pt = 0.9 * P + 0.1 * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    return (g["game"], g["gid"]), {"base": [lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"])[0] for t in T],
                                   "S3": [lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=1.4)[0] for t in T]}
if __name__ == "__main__":
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    with Pool(8) as pool: EV = dict(pool.map(work, grids, chunksize=4))
    start = {}
    for p in glob.glob("../data/lf/fdj/json/*.json.gz"):
        d = json.load(gzip.open(p))
        try: s = json.loads(d["state"])["gridResolver"]["startValidationDate"]
        except Exception: continue
        start[(int(d["game"]), int(d["gid"]))] = dt.datetime.fromisoformat(s.replace("Z", "+00:00")).astimezone(PAR)
    sat = collections.defaultdict(list)
    for g in grids:
        e = dt.datetime.fromisoformat(g["close"].replace("Z", "+00:00")).astimezone(PAR); s = start.get((g["game"], g["gid"]))
        t = (e - dt.timedelta(days=(e.weekday() - 5) % 7)).replace(hour=15, minute=0, second=0, microsecond=0)
        if s and s <= t < e: sat[t.date()].append((g["game"], g["gid"]))
    print(f"{len(sat)} samedis avec au moins une grille LF7/LF8 ouverte à 15 h (historique incomplet)")
    for tau in (1.0, 1.2, 1.5, 2.0):
        spend, gain_b, gain_s = [], [], []
        for day, ids in sat.items():
            # tickets de toutes les grilles du samedi, par espérance S3 décroissante, seuil τ, 16 au plus
            pool_t = sorted(((EV[i]["S3"][k], EV[i]["base"][k]) for i in ids for k in range(16)), reverse=True)
            chosen = [x for x in pool_t if x[0] >= tau][:16]
            spend.append(len(chosen)); gain_b.append(sum(b for _, b in chosen) - len(chosen)); gain_s.append(sum(s for s, _ in chosen) - len(chosen))
        print(f"τ = {tau:3.1f} : dépense moyenne {np.mean(spend):4.1f} €/samedi (0 € {np.mean(np.array(spend)==0):4.0%} des samedis), "
              f"gain espéré {52*np.mean(gain_b):+5.0f} €/an (S3 {52*np.mean(gain_s):+5.0f} €/an), espérance par euro joué base "
              f"{(np.sum(gain_b)+np.sum(spend))/max(np.sum(spend),1):.2f}")
