"""Nombre de tickets par grille qui maximise la croissance logarithmique E[ln(1 + (gain − k)/B)] (bankroll B = 1 000 €),
gains simulés par lf_sim (modèle calibré, plancher de 3 concurrents par combinaison pour rester prudent)."""
import sys, numpy as np
sys.path.insert(0, "."); import grids_now, lf_sim, lf_fast
import backtest_lf as B
B.K = 40
B.ev_fast = lambda t, p, q, S, m: lf_fast.ev(t, np.asarray(p), np.asarray(q), S, m, kappa=1.15, beta=0.976, floor=3)
rng = np.random.default_rng(20261022); BANK = 1000.0
for game in (7, 8, 12):
    g, p, q, names = grids_now.load(game); S, mf = grids_now.SETUP[game]; masses = mf(S); P, Q = np.array(p), np.array(q)
    tk = B.choose(p, q, S, masses); tickets = [t for _, t in tk]
    out = []
    for k in (1, 2, 5, 10, 20, 40):
        gain = lf_sim.simulate(tickets[:k], P, Q, S, masses, 300000, rng, sigma=0.0) 
        g_log = np.mean(np.log1p((gain - k) / BANK))
        out.append(f"k={k:>2}: EV {gain.mean()/k:.2f}, P(gain>0) {np.mean(gain>0):.3f}, croissance log {g_log*1e4:+.1f}‱")
    print(f"LF{game}: " + " | ".join(out))
