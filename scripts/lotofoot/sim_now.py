import sys, numpy as np
sys.path.insert(0, "."); import grids_now, lf_sim, lf_fast
import backtest_lf as B
rng = np.random.default_rng(20261022)
for game in (7, 8, 12, 15):
    g, p, q, names = grids_now.load(game); S, mf = grids_now.SETUP[game]; masses = mf(S)
    P, Q = np.array(p), np.array(q)
    tk = B.choose(p, q, S, masses)
    evs = [e for e, _ in tk]; tickets = [t for _, t in tk]
    gain = lf_sim.simulate(tickets, P, Q, S, masses, 400000, rng)
    cost = len(tickets)
    print(f"LF{game}: {cost} tickets, EV annoncée moyenne {np.mean(evs):.2f} (de {min(evs):.2f} à {max(evs):.2f}) ; simulée {gain.mean()/cost:.2f} ; "
          f"P(gain > 0) {np.mean(gain>0):.3f} ; P(gain ≥ mise) {np.mean(gain>=cost):.3f} ; médiane {np.median(gain):.1f} € ; p99 {np.percentile(gain,99):.0f} €")
    for t in tickets[:3]: print("     ", "".join("1N2"[x] for x in t))
