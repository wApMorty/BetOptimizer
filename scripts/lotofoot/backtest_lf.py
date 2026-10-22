"""Backtest sur les grilles passées (copies Wayback) : gains réellement versés.

PROTOCOLE FIGÉ LE 2026-10-22, AVANT LECTURE :
- probabilités de décision : cotes d'AVANT-match de football-data (Pinnacle PSH, sinon moyenne AvgH, sinon B365H), méthode puissance ;
- public : q ∝ p^1,670 · e^{0,075·nul} (gelé), concurrents corrigés κ = 1,15, β = 0,976 (calibration du jour) ;
- masses de décision : 70 % de S (S tiré des rangs 2 à 4 ou du rang 2, voir calib_fit.stakes) ; rang 1 sans garantie (0,28 S
  ou 0,315 S), donc prudent ;
- stratégie : K tickets simples distincts par grille, choisis glouton (meilleure espérance, puis meilleure espérance parmi les
  tickets différant d'au moins 2 signes des tickets déjà pris), K = 10 ; aucun seuil, aucune autre configuration ;
- gain réalisé : au rang atteint, gain unitaire publié × W / (W + 1) (notre ticket s'ajoute aux W gagnants) ; rang sans
  gagnant publié : masse de décision (0,28 S ou 0,315 S) ; mise 1 € par ticket ;
- intervalle : bootstrap des grilles (10 000 tirages, graine 20261022).
"""
import json, sys, math, itertools, numpy as np
sys.path.insert(0, "."); import lf_ev, lf_fast, fd_match
from calib import qmodel
from calib_fit import stakes
from market import power
KAPPA, BETA, K = 1.15, 0.976, 10
def prematch(fdrow):
    for cols in (("PSH", "PSD", "PSA"), ("AvgH", "AvgD", "AvgA"), ("B365H", "B365D", "B365A")):
        try: o = [float(fdrow[c]) for c in cols]
        except (KeyError, ValueError, TypeError): continue
        if 1.0 <= sum(1 / x for x in o) <= 1.15: return power(o)
    return None
def ev_fast(t, p, q, S, masses):
    return lf_fast.ev(t, np.asarray(p), np.asarray(q), S, masses, kappa=KAPPA, beta=BETA)
def choose(p, q, S, masses):
    n = len(p); picked = []
    starts = [[int(np.argmax(x)) for x in p], [int(np.argmax([p[i][o] / q[i][o] for o in range(3)])) for i in range(n)]]
    cache = {}
    def ev(t):
        k = tuple(t)
        if k not in cache: cache[k] = ev_fast(t, p, q, S, masses)
        return cache[k]
    while len(picked) < K:
        best = None
        for t0 in starts + [list(x) for x in picked]:
            t = list(t0)
            if any(sum(a != b for a, b in zip(t, u)) < 2 for u in picked):   # point de départ trop proche : on le perturbe
                t = [(t[i] + 1) % 3 if i == len(picked) % n else t[i] for i in range(n)]
            cur = ev(t) if all(sum(a != b for a, b in zip(t, u)) >= 2 for u in picked) else -1
            for _ in range(4):
                imp = False
                for i in range(n):
                    for o in range(3):
                        if o == t[i]: continue
                        t2 = list(t); t2[i] = o
                        if any(sum(a != b for a, b in zip(t2, u)) < 2 for u in picked): continue
                        v = ev(t2)
                        if v > cur + 1e-12: t, cur, imp = t2, v, True
                if not imp: break
            if cur > -1 and (best is None or cur > best[0]): best = (cur, t)
        picked.append(best[1]); 
    return [(cache[tuple(t)], t) for t in picked]
if __name__ == "__main__":
    grids = json.load(open(sys.argv[1])); rows = fd_match.load_fd()
    res = []
    for g in grids:
        S = stakes(g); n = len(g["ms"])
        if S is None or max(r["good"] for r in g["ranks"]) != n: continue
        p = []
        for m in g["matches"]:
            x, _ = fd_match.match(rows, m["home"], m["away"], __import__("datetime").date.fromisoformat(g["date"]), before=6, after=6)
            p.append(prematch(x["r"]) if x else None)
        if any(v is None for v in p): continue
        q = [qmodel(v) for v in p]
        masses = ([0.28 * S] + [0.14 * S] * 3) if g["game"] == 15 else [0.315 * S, 0.385 * S]
        R = len(masses); outcome = [m["o"] for m in g["ms"]]
        by_good = {r["good"]: r for r in g["ranks"]}
        tickets = choose(p, q, S, masses); won = 0.0; hits = []
        for ev, t in tickets:
            good = sum(a == b for a, b in zip(t, outcome)); rank = n - good
            if rank < R:
                r = by_good.get(good)
                if r and r.get("winners"): gain = r["gain"] * r["winners"] / (r["winners"] + 1)
                else: gain = masses[rank]
                won += gain; hits.append((rank + 1, round(gain, 1)))
        evs = [e for e, _ in tickets]
        res.append({"grid": f"{g['game']}-{g['date']}-{g['num']}", "cost": len(tickets), "won": won, "ev_mean": float(np.mean(evs)), "hits": hits})
        print(f"{res[-1]['grid']:18s} EV moyenne annoncée {np.mean(evs):.2f}  gagné {won:9.1f} € pour {len(tickets)} €  {hits}", flush=True)
    json.dump(res, open(sys.argv[2], "w"), indent=1)
    cost = sum(r["cost"] for r in res); won = sum(r["won"] for r in res)
    rng = np.random.default_rng(20261022); B = []
    for _ in range(10000):
        idx = rng.integers(0, len(res), len(res)); B.append(sum(res[i]["won"] for i in idx) / sum(res[i]["cost"] for i in idx) - 1)
    print(f"\n{len(res)} grilles, mise {cost} €, gains {won:.1f} €, ROI {won/cost-1:+.1%} [IC 95 % {np.percentile(B,2.5):+.1%} ; {np.percentile(B,97.5):+.1%}]")
    print(f"EV moyenne annoncée : {np.mean([r['ev_mean'] for r in res]):.2f}")
    for game in (7, 15):
        sub = [r for r in res if r["grid"].startswith(f"{game}-")]
        if sub: print(f"  LF{game} : {len(sub)} grilles, ROI {sum(r['won'] for r in sub)/sum(r['cost'] for r in sub)-1:+.1%}, grilles avec un gain : {sum(1 for r in sub if r['won']>0)}")
