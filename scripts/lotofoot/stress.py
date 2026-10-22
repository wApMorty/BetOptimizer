import sys, numpy as np
sys.path.insert(0, "."); import grids_now, lf_ev
from calib import qmodel
SC = [("modèle brut", dict()), ("calibré κ=1,15 β=0,976", dict(kappa=1.15, beta=0.976)),
      ("calibré + ×2,1 sous λ=2", dict(kappa=1.15, beta=0.976, low_mult=2.1)),
      ("calibré + bruit σ=0,6", dict(kappa=1.15, beta=0.976, sigma=0.6)),
      ("pessimiste : ×2 partout, ×4 sous λ=2", dict(kappa=2.0, low_mult=2.0))]
for game in (7, 8, 12, 15):
    g, p, q, names = grids_now.load(game); S, mf = grids_now.SETUP[game]; masses = mf(S); n = len(p)
    fav = [int(np.argmax(x)) for x in p]; ratio = [int(np.argmax([p[i][o] / q[i][o] for o in range(3)])) for i in range(n)]
    best = lf_ev.optimize(p, q, 2.0 * S, masses, [fav, ratio])    # ticket choisi sous l'hypothèse pessimiste (2× plus de concurrents)
    t = best[1]; print(f"LF{game} ticket {''.join('1N2'[x] for x in t)}")
    for name, kw in SC: print(f"   {name:38s} EV {lf_ev.ev_corrected(t, p, q, S, masses, **kw):.3f}")
