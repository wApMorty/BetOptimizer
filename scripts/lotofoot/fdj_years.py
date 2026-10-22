"""Gain réalisé par année et concentration des gains (stratégie principale, LF7 et LF8, entraînement et test)."""
import sys, json, numpy as np, collections
sys.path.insert(0, "."); import lf_fast
import backtest_fdj as BF
grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
prm = json.load(open("../data/lf/fdj_params_7_8.json"))
res = BF.run(grids, prm, "tout")
by = collections.defaultdict(lambda: [0, 0.0, 0, 0.0])
for r in res:
    y = r["close"][:4]; b = by[y]; b[0] += r["cost"]; b[1] += r["won"]; b[2] += r["won"] > 0; b[3] += r["ev"] * r["cost"]
for y in sorted(by):
    c, w, k, e = by[y]; print(f"{y} : {c//10:3d} grilles, misé {c:5d} €, gagné {w:8.0f} €, ROI {w/c-1:+7.1%}, espérance annoncée {e/c:.2f}, grilles payées {k}")
wins = sorted((r["won"] for r in res), reverse=True); tot = sum(wins); cost = sum(r["cost"] for r in res)
print(f"total {len(res)} grilles, misé {cost} €, gagné {tot:.0f} €, ROI {tot/cost-1:+.1%}")
print("5 plus gros gains :", [round(x) for x in wins[:5]], f"→ {sum(wins[:5])/tot:.0%} du total")
print(f"ROI sans le plus gros gain : {(tot-wins[0])/cost-1:+.1%} ; sans les 3 plus gros : {(tot-sum(wins[:3]))/cost-1:+.1%}")
json.dump(res, open("../data/lf/bt_fdj_all_7_8.json", "w"))
