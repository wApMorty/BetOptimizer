import sys, numpy as np
sys.path.insert(0, "."); import grids_now, lf_fast
import backtest_lf as B
for game in (7, 8, 12, 15):
    g, p, q, names = grids_now.load(game); S, mf = grids_now.SETUP[game]; masses = mf(S); P, Q = np.array(p), np.array(q)
    for fl in (0, 1, 3, 10):
        B.ev_fast = lambda t, p_, q_, S_, m_, fl=fl: lf_fast.ev(t, np.asarray(p_), np.asarray(q_), S_, m_, kappa=1.15, beta=0.976, floor=fl)
        tk = B.choose(p, q, S, masses)          # tickets choisis en connaissant le plancher
        evs = [e for e, _ in tk]
        # robustesse : tickets choisis SANS plancher, évalués avec ce plancher
        print(f"LF{game} plancher {fl:>2} : 10 tickets choisis avec plancher, EV moyenne {np.mean(evs):.2f} (min {min(evs):.2f})  premier {''.join('1N2'[x] for x in tk[0][1])}")
