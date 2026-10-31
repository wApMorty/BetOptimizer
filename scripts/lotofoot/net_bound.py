"""Certificat du glouton de `net_core` : borne supérieure par relaxation linéaire, sur des grilles d'entraînement (2026-10-31).

Programme, pour k tickets sur une grille, x_t ≥ 0 réels :
    max Σ_o P(o) [ M_0 y_0(o) + M_1 y_1(o) ]
    y_0(o) ≤ F_0(c) + dF_0(c) (x_o − c)                      pour c dans TANGENTS,
    y_1(o) ≤ F_1(c) + dF_1(c) (Σ_{t à une erreur de o} x_t − c)  pour c dans TANGENTS,    Σ_t x_t ≤ k.
F étant concave, chaque corde prolongée la majore aux entiers : l'optimum de ce programme majore tout ensemble entier de k tickets.
Écart = (borne − glouton) / glouton. Résolu par HiGHS (`scipy.optimize.linprog`, méthode des points intérieurs).

Usage, depuis scripts/lotofoot : `~/.local/share/edge/etudes/venv/bin/python net_bound.py > ../data/lf/net_bound.txt 2> ../data/lf/net_bound.log`
"""
import sys, json, time, datetime as dt, numpy as np
from multiprocessing import Pool
from scipy import sparse
from scipy.optimize import linprog
sys.path.insert(0, ".")
import backtest_fdj as BF, net_core as C
from frontier import PRM

TANGENTS = [0, 1, 2, 3, 5, 8, 12, 16, 24, 32]
SIZES = [4, 16, 64]
START = time.time()


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - START:6.0f} s] {message}", file=sys.stderr, flush=True)


def bound(table, k):
    size = len(table.prob)
    F = [np.concatenate([np.zeros((size, 1)), np.cumsum(d, axis=1)], axis=1) for d in table.dF]
    rows, cols, vals, rhs = [], [], [], []
    row = 0
    arange = np.arange(size)
    for r in (0, 1):
        for c in TANGENTS:
            slope = table.dF[r][:, c]
            intercept = F[r][:, c] - c * slope
            # y_r(o) − dF · C_r(o) ≤ intercept
            rows.append(row + arange); cols.append((1 + r) * size + arange); vals.append(np.ones(size))
            if r == 0:
                rows.append(row + arange); cols.append(arange); vals.append(-slope)
            else:
                rows.append(np.repeat(row + arange, table.N.shape[1])); cols.append(table.N.ravel())
                vals.append(-np.repeat(slope, table.N.shape[1]))
            rhs.append(intercept)
            row += size
    rows.append(np.full(size, row)); cols.append(arange); vals.append(np.ones(size)); rhs.append(np.array([float(k)]))
    A = sparse.csr_matrix((np.concatenate(vals), (np.concatenate(rows), np.concatenate(cols))), shape=(row + 1, 3 * size))
    cost = np.concatenate([np.zeros(size), -table.M[0] * table.prob, -table.M[1] * table.prob])
    result = linprog(cost, A_ub=A, b_ub=np.concatenate(rhs), bounds=(0, None), method="highs-ipm")
    return -result.fun


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    table = C.Table(C.s3_probabilities(P, Q), Q, S, M, prm, 1.4)
    out = []
    for k in SIZES:
        seq, _ = table.greedy(k, copies=True)
        out.append((g["game"], k, table.walk(seq)[-1], bound(table, k)))
    return out


if __name__ == "__main__":
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8) and g["close"] < "2024"]
    rng = np.random.default_rng(31)
    sample = [grids[i] for i in rng.choice(len(grids), 24, replace=False)]
    log(f"{len(sample)} grilles d'entraînement tirées, {len(SIZES)} tailles")
    done = []
    with Pool(4) as pool:
        for i, r in enumerate(pool.imap(work, sample), 1):
            done.extend(r)
            log(f"grilles {i} / {len(sample)}, reste environ {(time.time() - START) / i * (len(sample) - i):.0f} s")
    print("jeu  k  grilles  écart moyen  écart maximal")
    for game in (7, 8):
        for k in SIZES:
            gaps = [(b - v) / v for g, kk, v, b in done if g == game and kk == k]
            if gaps:
                print(f"LF{game} {k:3d}  {len(gaps):3d}   {np.mean(gaps):7.2%}   {np.max(gaps):7.2%}")
