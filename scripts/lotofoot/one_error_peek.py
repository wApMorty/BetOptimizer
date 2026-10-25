"""Regard préalable (question de Paul du 2026-10-25) : combien de tickets retenus ont un gain « une erreur » sous la mise ?

Pour chacune des 409 grilles LF7/LF8 de l'étude FDJ, les 16 tickets de la recherche de production (comme `budget_tau.py`) sont
décomposés par rang en S3 : chance P_r et gain moyen sachant le rang G_r = E[gain · 1{rang r}] / P_r (comme la page `/lotofoot`).
Les tickets sont mis en cache (`../data/lf/one_error_tickets.json`) pour l'étude elle-même. Aucun résultat de match n'est lu.

Usage, depuis scripts/lotofoot : `python3 one_error_peek.py` (journal sur stderr).
"""
import sys, json, os, time, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM

PER_GRID, SHRINK, LOW_MULT, PRICE = 16, 0.1, 1.4, 1.0
CACHE = "../data/lf/one_error_tickets.json"
T0 = time.time()


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')} +{time.time() - T0:6.0f} s] {msg}", file=sys.stderr, flush=True)


def ev_ranks(t, P, Q, S, masses, kappa, beta, phi, low_mult=1.0, low_cut=3.0):
    """Décomposition par rang de `lf_fast.ev_p` : [(P_r, E[gain · 1{rang r}])] pour r = 0 (sans faute), 1 (une erreur)…"""
    n, R = P.shape[0], len(masses)
    D, E = lf_fast.configs(n, R)
    O = (np.asarray(t)[None, :] + D) % 3
    idx = np.arange(n)[None, :]
    prob = np.prod(P[idx, O], axis=1)
    a = Q[idx, O]
    dist = np.zeros((len(E), n + 1)); dist[:, 0] = 1.0
    for i in range(n):
        x = a[:, i:i + 1]
        dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    lam = S * dist[np.arange(len(E)), n - E]
    mu = kappa * np.power(np.maximum(lam, 1e-300), beta) + phi * S / 3 ** n
    mu = np.where(lam < low_cut, mu * low_mult, mu)
    share = np.where(mu < 1e-9, 1.0, -np.expm1(-mu) / np.maximum(mu, 1e-300))
    gain = prob * np.asarray(masses)[E] * share
    return [(float(prob[E == r].sum()), float(gain[E == r].sum())) for r in range(R)]


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    T = B16.greedy(P, Q, S, M, prm, PER_GRID, 1)
    Pt = (1 - SHRINK) * P + SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    rows = []
    for t in T:
        ranks = ev_ranks(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=LOW_MULT)
        base = lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"])[0]
        rows.append({"t": [int(x) for x in t], "s3": sum(c for _, c in ranks) / PRICE, "base": base,
                     "p": [p for p, _ in ranks], "c": [c for _, c in ranks]})
    rows.sort(key=lambda r: -r["s3"])
    return {"game": g["game"], "gid": g["gid"], "close": g["close"], "tickets": rows}


def load_or_build():
    if os.path.exists(CACHE):
        log(f"tickets lus dans le cache {CACHE}")
        return json.load(open(CACHE))
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    log(f"recherche des 16 tickets sur {len(grids)} grilles (8 processus)")
    out = []
    with Pool(8) as pool:
        for k, r in enumerate(pool.imap(work, grids, chunksize=2), 1):
            out.append(r)
            if k % 20 == 0 or k == len(grids):
                el = time.time() - T0
                log(f"  {k} / {len(grids)} grilles, reste ≈ {el / k * (len(grids) - k):.0f} s")
    json.dump(out, open(CACHE, "w"))
    return out


if __name__ == "__main__":
    data = load_or_build()
    for tau in (1.4, 1.7, 1.8, 2.0):
        for game in (7, 8):
            sel = [t for g in data if g["game"] == game for t in g["tickets"] if t["s3"] >= tau]
            g2 = np.array([t["c"][1] / t["p"][1] if t["p"][1] > 0 else 0 for t in sel])
            low = g2 < PRICE
            share_r2 = np.array([t["c"][1] / (t["s3"] * PRICE) for t in sel])
            ev_low = np.array([t["s3"] for t in sel])[low]
            print(f"τ = {tau} LF{game} : {len(sel)} tickets ≥ τ, dont {low.sum()} ({low.mean():.0%}) à gain « une erreur » < 1 € ; "
                  f"part du rang 2 dans l'espérance : {np.median(share_r2):.1%} en médiane ; "
                  f"espérance S3 de ces tickets : médiane {np.median(ev_low) if low.any() else float('nan'):.2f}")
