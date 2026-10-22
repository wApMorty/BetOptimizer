"""Diagnostic du backtest : pour chaque grille, nombre de bons du meilleur ticket et probabilité (marché) d'au moins un rang payé."""
import json, sys, datetime as dt, itertools, numpy as np
sys.path.insert(0, "."); import fd_match, backtest_lf as B
from calib import qmodel
from calib_fit import stakes
grids = json.load(open("../data/lf/grids_ok.json")); rows = fd_match.load_fd()
tot_exp, tot_obs, tot_exp_any = 0, 0, 0
for g in grids:
    S = stakes(g); n = len(g["ms"])
    if S is None or max(r["good"] for r in g["ranks"]) != n: continue
    p = []
    for m in g["matches"]:
        x, _ = fd_match.match(rows, m["home"], m["away"], dt.date.fromisoformat(g["date"]), before=6, after=6)
        p.append(B.prematch(x["r"]) if x else None)
    if any(v is None for v in p): continue
    q = [qmodel(v) for v in p]
    masses = ([0.28 * S] + [0.14 * S] * 3) if g["game"] == 15 else [0.315 * S, 0.385 * S]
    R = len(masses); outcome = [m["o"] for m in g["ms"]]
    tk = [t for _, t in B.choose(p, q, S, masses)]
    goods = [sum(a == b for a, b in zip(t, outcome)) for t in tk]
    # Monte Carlo sur les probabilités de clôture (meilleure estimation de la vérité) : P(au moins un ticket payé)
    pc = np.array([m["p"] for m in g["ms"]]); rng = np.random.default_rng(1)
    U = rng.random((200000, n)); O = (U[:, :, None] > np.cumsum(pc, axis=1)[None, :, :-1]).sum(axis=2)
    G = (np.array(tk)[None, :, :] == O[:, None, :]).sum(axis=2)
    pany = np.mean((G >= n - R + 1).any(axis=1)); nexp = np.mean((G >= n - R + 1).sum(axis=1))
    tot_exp_any += pany; tot_exp += nexp; tot_obs += sum(x >= n - R + 1 for x in goods)
    fav = sum(int(np.argmax(m["p"])) == m["o"] for m in g["ms"])
    print(f"{g['game']}-{g['date']}  n={n}  bons du meilleur ticket {max(goods)} (seuil payé {n-R+1})  favoris sortis {fav}/{n}  P(≥1 payé) {pany:.3f}")
print(f"grilles avec un ticket payé attendues {tot_exp_any:.2f} ; tickets payés attendus {tot_exp:.2f} ; observés {tot_obs}")
