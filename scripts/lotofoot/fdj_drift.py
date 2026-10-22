"""Pour les signes choisis par la stratégie : probabilité d'avant-match (vendredi/mardi) contre clôture (Pinnacle, sinon moyenne),
selon l'écart avant-match − public. Si la clôture se rapproche du public, l'avantage vient en partie de cotes périmées."""
import sys, json, datetime as dt, numpy as np, collections
sys.path.insert(0, "."); import lf_fast, fd_match
import backtest_fdj as BF
import backtest_lf as B
from market import power
grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
prm = json.load(open("../data/lf/fdj_params_7_8.json")); rows = fd_match.load_fd()
B.K = 10
B.ev_fast = lambda t, p, q, S, m: lf_fast.ev(t, np.asarray(p), np.asarray(q), S, m, kappa=prm["kappa"], beta=prm["beta"], phi=prm["phi"])
def close(r):
    for c in (("PSCH", "PSCD", "PSCA"), ("AvgCH", "AvgCD", "AvgCA")):
        try: o = [float(r[x]) for x in c]
        except (KeyError, ValueError, TypeError): continue
        if 1.0 <= sum(1 / x for x in o) <= 1.15: return power(o)
    return None
acc = collections.defaultdict(lambda: [0, 0.0, 0.0, 0.0, 0])   # n, Σ p avant, Σ p clôture, Σ q, réussites
for g in grids:
    if any(s and s.startswith(("PSC", "AvgC", "B365C")) for s in g["src"]): continue   # avant-match = clôture : sans intérêt
    tk = [t for _, t in B.choose(g["p"], g["q"], g["S"], BF.masses(g))]
    for i, ((h, a), st) in enumerate(zip(g["teams"], g["start"])):
        x, _ = fd_match.match(rows, h, a, dt.date.fromisoformat(st[:10]), before=1, after=1)
        pc = close(x["r"]) if x else None
        if pc is None: continue
        for o in {t[i] for t in tk}:
            d = g["p"][i][o] - g["q"][i][o]
            k = "p−q < −0,05" if d < -0.05 else "−0,05 ≤ p−q < 0,05" if d < 0.05 else "p−q ≥ 0,05"
            v = acc[k]; v[0] += 1; v[1] += g["p"][i][o]; v[2] += pc[o]; v[3] += g["q"][i][o]; v[4] += (g["o"][i] == o)
for k in ("p−q < −0,05", "−0,05 ≤ p−q < 0,05", "p−q ≥ 0,05"):
    n, a, c, q, w = acc[k]
    print(f"{k:20s} : {n:5d} signes ; avant-match {a/n:.4f}, clôture {c/n:.4f} (écart {100*(c-a)/n:+.2f} pt), public {q/n:.4f}, fréquence réelle {w/n:.4f}")
