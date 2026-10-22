"""Diagnostic du backtest LF7/LF8 : (1) grilles payées attendues sous p (marché) contre observées ; (2) calibration des
signes CHOISIS : fréquence de réussite réelle des signes de nos tickets contre la probabilité du marché, selon l'écart p − q."""
import sys, json, numpy as np, collections
sys.path.insert(0, "."); import lf_fast
import backtest_fdj as BF
import backtest_lf as B
SPLIT = "2024-01-01"
grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
prm = json.load(open("../data/lf/fdj_params_7_8.json"))
B.K = 10
B.ev_fast = lambda t, p, q, S, m: lf_fast.ev(t, np.asarray(p), np.asarray(q), S, m, kappa=prm["kappa"], beta=prm["beta"], phi=prm["phi"])
rng = np.random.default_rng(7)
for period in ("entr.", "test"):
    sub = [g for g in grids if (g["close"] >= SPLIT) == (period == "test")]
    exp_paid = obs_paid = 0; cal = collections.defaultdict(lambda: [0.0, 0, 0])   # tranche p−q → [Σp, réussites, n]
    for g in sub:
        n = g["n"]; P = np.array(g["p"]); M = BF.masses(g); R = len(M)
        tk = [t for _, t in B.choose(g["p"], g["q"], g["S"], M)]
        U = rng.random((20000, n)); O = (U[:, :, None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
        G = (np.array(tk)[None, :, :] == O[:, None, :]).sum(axis=2)
        exp_paid += np.mean((G >= n - R + 1).any(axis=1))
        obs_paid += any(sum(a == b for a, b in zip(t, g["o"])) >= n - R + 1 for t in tk)
        for i in range(n):                       # signes distincts choisis sur ce match
            for o in {t[i] for t in tk}:
                d = g["p"][i][o] - g["q"][i][o]
                k = "p−q < −0,05" if d < -0.05 else "−0,05 ≤ p−q < 0,05" if d < 0.05 else "0,05 ≤ p−q < 0,15" if d < 0.15 else "p−q ≥ 0,15"
                c = cal[k]; c[0] += g["p"][i][o]; c[1] += (g["o"][i] == o); c[2] += 1
    print(f"{period} : {len(sub)} grilles ; grilles payées attendues {exp_paid:.1f}, observées {obs_paid}")
    for k in ("p−q < −0,05", "−0,05 ≤ p−q < 0,05", "0,05 ≤ p−q < 0,15", "p−q ≥ 0,15"):
        if k in cal:
            sp, w, m = cal[k]; se = (sp / m * (1 - sp / m) / m) ** 0.5
            print(f"   signes choisis {k:20s} : {m:4d}, probabilité moyenne du marché {sp/m:.3f}, fréquence réelle {w/m:.3f} (± {se:.3f})")
