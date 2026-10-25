"""Répartition arrondie au pourcent (prono-match) contre répartition exacte (FDJ) : combien de tickets passent τ « à tort » ?

Question de Paul (2026-10-25) : beaucoup de tickets au-dessus du seuil en production, même calculés moins d'une heure avant la
clôture. Hypothèse testée : prono-match affiche des parts entières (« 79·13·8 ») ; la recherche choisit de préférence les signes dont
la part a été arrondie vers le bas (malédiction du vainqueur), et leur espérance annoncée dépasse leur valeur réelle.

Protocole (ex ante, aucun résultat de match) : les 409 grilles LF7/LF8 de l'étude FDJ ; q arrondi au pourcent comme le publie
prono-match puis traité comme en production (`public_split` : part nulle comptée 0,5 %, renormalisation) ; recherche de production
(16 tickets, μ = 10) sur q arrondi ; pour chaque ticket, espérance S3 annoncée (q arrondi) et réelle (q exact, mêmes tickets).
Comparaison : tickets au-dessus de τ = 1,7 et 1,8, annoncés contre réels, et contre la recherche sur q exact (backtest du §49).
"""
import sys, json, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM

PER_GRID = 16


def production_q(q):
    """Parts entières de prono-match, traitées comme `public_split` (part nulle → 0,5 %)."""
    shares = np.maximum(np.round(np.asarray(q) * 100), 0.5)
    return shares / shares.sum(axis=1, keepdims=True)


def s3(t, P, Q, S, M, prm):
    Pt = 0.9 * P + 0.1 * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    return lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=1.4)[0]


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    Qr = production_q(Q)
    T = B16.greedy(P, Qr, S, M, prm, PER_GRID, 1)
    shown = [s3(t, P, Qr, S, M, prm) for t in T]
    # Valeur réelle des mêmes tickets : les autres joueurs ont misé selon q exact.
    real = [s3(t, P, Q, S, M, prm) for t in T]
    return {"game": g["game"], "shown": shown, "real": real}


if __name__ == "__main__":
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    ref = json.load(open("../data/lf/weekly_budget_ev.json"))           # recherche sur q exact (§47, §49)
    with Pool(8) as pool:
        out = pool.map(work, grids, chunksize=4)
    for game in (7, 8):
        rows = [(o, r) for o, r in zip(out, ref) if o["game"] == game]
        print(f"\nLF{game} ({len(rows)} grilles)")
        for tau in (1.7, 1.8):
            shown = np.array([sum(v >= tau for v in o["shown"]) for o, _ in rows])
            true_of_shown = np.array([sum(v >= tau for v, w in zip(o["real"], o["shown"]) if w >= tau) for o, _ in rows])
            exact = np.array([sum(v >= tau for v in r["s3"]) for _, r in rows])
            print(f"  τ = {tau} : tickets au-dessus par grille — annoncés (q arrondi) {shown.mean():.2f}, dont vraiment au-dessus "
                  f"{true_of_shown.mean():.2f} ; recherche sur q exact {exact.mean():.2f} ; grilles sans ticket annoncé "
                  f"{np.mean(shown == 0):.0%} (exact {np.mean(exact == 0):.0%})")
        gap = [w - v for o, _ in rows for v, w in zip(o["real"], o["shown"]) if w >= 1.7]
        if gap:
            print(f"  écart annoncé − réel des tickets annoncés ≥ 1,7 : moyenne {np.mean(gap):+.3f}, médiane {np.median(gap):+.3f}, "
                  f"90e centile {np.percentile(gap, 90):+.3f}")
