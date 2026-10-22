"""Contrôle de cohérence : espérance moyenne d'une grille du public, pondérée par q (doit valoir ≈ 0,70 hors garantie)."""
import sys, itertools, math, numpy as np
sys.path.insert(0, "."); import grids_now, lf_ev
game = int(sys.argv[1])
g, p, q, names = grids_now.load(game); S, mf = grids_now.SETUP[game]; masses = mf(S); n = len(p)
tot = 0.0
for c in itertools.product(range(3), repeat=n):
    qc = math.prod(q[i][c[i]] for i in range(n))
    if qc < 1e-7: continue
    tot += qc * lf_ev.ev_ticket(list(c), p, q, S, masses)[0]
print(f"LF{game} : espérance moyenne des grilles du public {tot:.3f} (masse totale / mises = {sum(masses)/S:.3f})")
