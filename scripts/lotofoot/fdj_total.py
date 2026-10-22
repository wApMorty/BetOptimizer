"""Bilan de toutes les grilles appariées (entraînement et test), par jeu : gain réalisé, intervalle (bootstrap des semaines),
concentration, et part du gain venant du meilleur résultat."""
import sys, json, datetime as dt, collections, numpy as np
sys.path.insert(0, "."); import backtest_fdj as BF
grids = json.load(open("../data/lf/grids_fdj.json"))
res = json.load(open("../data/lf/bt_fdj_all_7_8.json"))
g1215 = [g for g in grids if g["game"] in (12, 15)]
res += BF.run(g1215, json.load(open("../data/lf/fdj_params_12_15.json")), "tout")
json.dump(res, open("../data/lf/bt_fdj_all.json", "w"))
def summary(rs, label):
    c = sum(r["cost"] for r in rs); w = sum(r["won"] for r in rs)
    weeks = collections.defaultdict(list)
    for r in rs: weeks[dt.date.fromisoformat(r["close"][:10]).isocalendar()[:2]].append(r)
    keys = list(weeks); rng = np.random.default_rng(20261022); B = []
    for _ in range(10000):
        flat = [r for i in rng.integers(0, len(keys), len(keys)) for r in weeks[keys[i]]]
        B.append(sum(r["won"] for r in flat) / sum(r["cost"] for r in flat) - 1)
    top = max(r["won"] for r in rs)
    print(f"{label:10s} {len(rs):4d} grilles, misé {c:5d} €, gagné {w:7.0f} €, ROI {w/c-1:+8.1%} [IC 95 % {np.percentile(B,2.5):+.0%} ; {np.percentile(B,97.5):+.0%}], "
          f"EV annoncée {np.mean([r['ev'] for r in rs]):.2f}, grilles payées {sum(r['won']>0 for r in rs)}, sans le meilleur gain {(w-top)/c-1:+.0%}")
for gm in (7, 8, 12, 15): summary([r for r in res if r["game"] == gm], f"LF{gm}")
summary(res, "TOUS")
