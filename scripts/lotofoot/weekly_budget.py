"""Budget de 16 € par semaine sur toutes les grilles LF7 et LF8 (demande de Paul du 2026-10-24) : quel seuil τ ?

Protocole, figé avant le calcul (calculs ex ante : espérances du modèle, aucun résultat de match) :
- grilles : les 409 grilles LF7/LF8 appariées de l'étude (`grids_fdj.json`, vraie répartition FDJ, cotes d'avant-match) ; pour
  chacune, les 16 tickets de la recherche de production (gloutonne μ = 10, tickets distincts), évalués en base et en S3 ;
- semaine simulée : nombre de grilles tiré selon une loi de Poisson, λ7 = 3,2 LF7 et λ8 = 3,4 LF8 par semaine (numérotation FDJ
  2025-2026) ; grilles tirées avec remise dans le lot de leur formule ; elles arrivent une à une dans un ordre aléatoire ;
- règle : sur chaque grille, les tickets d'espérance S3 ≥ τ, les meilleurs d'abord, dans la limite de ce qui reste des 16 € de la
  semaine (et de 16 par grille) ;
- critère : gain net espéré S3 par semaine (Σ (EV S3 − 1) des tickets joués), maximal sur l'entraînement (grilles closes avant
  2024) ; le test (2024 et après) n'est regardé qu'après le choix ;
- configurations : τ de 1,0 à 4,0 par pas de 0,1 ; sensibilité au rythme de 2022 (λ7 = 5,1, λ8 = 2,3).
Limites : grilles appariées seulement (championnats couverts par football-data ; coupes et sélections absentes) ; partage entre
nos propres tickets non déduit ; une semaine réelle n'est pas un tirage indépendant de grilles.
"""
import sys, json, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM

BUDGET, PER_GRID, WEEKS = 16, 16, 40000


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    T = B16.greedy(P, Q, S, M, prm, PER_GRID, 1)
    Pt = 0.9 * P + 0.1 * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    base = [lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"])[0] for t in T]
    s3 = [lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=1.4)[0] for t in T]
    order = np.argsort(s3)[::-1]
    return {"game": g["game"], "train": g["close"] < "2024", "s3": [s3[k] for k in order], "base": [base[k] for k in order]}


def simulate(pool, tau, lam7, lam8, seed, cap=BUDGET):
    rng = np.random.default_rng(seed)
    by = {7: [g for g in pool if g["game"] == 7], 8: [g for g in pool if g["game"] == 8]}
    spend, g_s3, g_base, capped, idle = [], [], [], 0, 0
    for _ in range(WEEKS):
        week = [by[7][i] for i in rng.integers(len(by[7]), size=rng.poisson(lam7))] + \
               [by[8][i] for i in rng.integers(len(by[8]), size=rng.poisson(lam8))]
        rng.shuffle(week)
        left, s, b = cap, 0.0, 0.0
        for g in week:
            for s3, base in zip(g["s3"], g["base"]):
                if left == 0 or s3 < tau: break
                left -= 1; s += s3 - 1; b += base - 1
        played = cap - left
        spend.append(played); g_s3.append(s); g_base.append(b); capped += left == 0; idle += played == 0
    n = WEEKS
    return {"tau": tau, "spend": np.mean(spend), "s3": np.mean(g_s3), "base": np.mean(g_base), "capped": capped / n, "idle": idle / n,
            "per_euro_s3": (np.sum(g_s3) + np.sum(spend)) / max(np.sum(spend), 1)}


def table(rows, title):
    print(f"\n{title}")
    print(" τ    dépense/sem.  gain S3/an  gain base/an  EV S3 par €  semaines au plafond  semaines sans jeu")
    for r in rows:
        print(f"{r['tau']:4.1f}  {r['spend']:6.1f} €    {52 * r['s3']:+7.0f} €   {52 * r['base']:+7.0f} €     {r['per_euro_s3']:.2f}"
              f"         {r['capped']:5.0%}              {r['idle']:5.0%}")


if __name__ == "__main__":
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    with Pool(8) as pool:
        ev = pool.map(work, grids, chunksize=4)
    json.dump(ev, open("../data/lf/weekly_budget_ev.json", "w"))
    train = [g for g in ev if g["train"]]; test = [g for g in ev if not g["train"]]
    print(f"grilles : {len(train)} d'entraînement, {len(test)} de test ; "
          f"LF7 {sum(g['game'] == 7 for g in ev)}, LF8 {sum(g['game'] == 8 for g in ev)}")
    taus = [round(1.0 + 0.1 * k, 1) for k in range(31)]
    rows = [simulate(train, t, 3.2, 3.4, seed=1) for t in taus]
    best = max(rows, key=lambda r: r["s3"])
    table(rows, "Entraînement, rythme 2025-2026 (3,2 LF7 + 3,4 LF8 par semaine)")
    print(f"\nSeuil retenu sur l'entraînement : τ = {best['tau']:.1f}")
    near = [best["tau"] + d for d in (-0.5, -0.2, 0.0, 0.2, 0.5)]
    table([simulate(test, round(t, 1), 3.2, 3.4, seed=2) for t in near], "Test (2024 et après), mêmes seuils autour du retenu")
    table([simulate(train, round(t, 1), 5.1, 2.3, seed=3) for t in near], "Sensibilité : rythme de 2022 (5,1 LF7 + 2,3 LF8)")
    table([simulate(train, 1.2, 3.2, 3.4, seed=4), simulate(train, 1.2, 3.2, 3.4, seed=4, cap=10 ** 6)],
          "Référence : τ = 1,2 avec le plafond de 16 € (premier arrivé), puis sans plafond")
