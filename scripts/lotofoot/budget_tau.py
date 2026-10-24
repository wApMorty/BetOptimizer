"""Abaque τ × budget mensuel (demande de Paul du 2026-10-24) : pour chaque budget, quel seuil τ, quel gain, quel risque ?

Protocole, figé avant le calcul (calculs ex ante : espérances et tirages du modèle gelé, aucun résultat de match) :
- grilles : les 409 grilles LF7/LF8 appariées de l'étude FDJ (`grids_fdj.json`, vraie répartition finale, cotes d'avant-match) ;
  pour chacune, les 16 tickets de la recherche de production (gloutonne μ = 10, tickets distincts d'au moins un signe), classés
  par espérance S3 décroissante, évalués en base et en S3 (p rapproché de q de 10 %, concurrents × 1,4 si λ < 3) ;
- tirages : pour chaque grille, 2 000 tirages des issues (selon p en base, selon p rapproché de q en S3) et des gagnants concurrents
  (binomiale négative du modèle gelé, comme `year_sim.py`) ; gain des k premiers tickets, partage entre nos propres tickets compris ;
- mois simulé : nombre de grilles tiré selon une loi de Poisson, λ7 = 3,2 et λ8 = 3,4 par semaine (numérotation FDJ 2025-2026)
  × 30,44 / 7 ; grilles tirées avec remise dans le lot de leur formule ; elles arrivent une à une dans un ordre aléatoire ;
- règle (séquentielle, celle du §47) : sur chaque grille, les tickets d'espérance S3 ≥ τ, les meilleurs d'abord, dans la limite de
  ce qui reste du budget du mois calendaire (aucun report d'un mois sur l'autre) et de 16 par grille ;
- année simulée : 12 mois ; 4 000 années, mêmes arrivées et mêmes tirages pour toutes les configurations (comparaisons appariées) ;
- configurations : τ de 1,0 à 4,0 par pas de 0,1 (31) × budget de 20 à 400 € par mois par pas de 20 € (20), soit 620 ;
- critère : pour chaque budget, τ qui maximise le gain net espéré S3 par an du modèle (Σ (EV S3 − 1) des tickets joués), sur
  l'entraînement (grilles closes avant 2024) ; le test (2024 et après) n'est regardé qu'après le choix ;
- affiché à côté, sans servir au choix (choix de Paul : « espérance + risque affiché ») : gain moyen simulé S3 (partage entre nos
  tickets compris), probabilité de perte sur un an, médiane et 10e centile du résultat annuel, plus grand repli dans l'année
  (au fil des grilles), part des mois au plafond.
Limites : répartition finale et non celle de l'heure du calcul ; grilles de championnat seulement (coupes et sélections absentes) ;
16 tickets au plus par grille (contrainte possible aux gros budgets) ; mois réels non indépendants ; aucun gain démontré (§37).

Usage, depuis scripts/lotofoot : `python3 budget_tau.py` (tickets et tirages mis en cache dans ../data/lf/budget_tau_cache.npz).
"""
import sys, json, os, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM

PER_GRID, NS, YEARS = 16, 2000, 4000
LAM = {7: 3.2 * 30.44 / 7, 8: 3.4 * 30.44 / 7}
TAUS = [round(1.0 + 0.1 * k, 1) for k in range(31)]
BUDGETS = list(range(20, 401, 20))
TH = 11.34                       # dispersion relative de la binomiale négative des concurrents LF7/LF8 (year_sim.py)
SHRINK, LOW_MULT = 0.1, 1.4      # scénario S3
CACHE = "../data/lf/budget_tau_cache.npz"
OUT = "../data/lf/budget_tau.json"


def prefix_gains(g, T, P, Q, rng, low_mult):
    """Gain brut des k premiers tickets (k = 0..16) sur NS tirages : matrice NS × 17.

    Rang r (n − r bons signes) : masse M_r partagée entre W concurrents et nos c_k tickets gagnants parmi les k premiers,
    gain = M_r · c_k / (W + c_k) ; W ~ binomiale négative de moyenne μ = κ λ^β + φ S / 3^n (× low_mult si λ < 3), λ = S · P(rang
    r selon la répartition du public) (`PROTOCOLE_FDJ.md`, §37)."""
    n = g["n"]; S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    O = (rng.random((NS, n))[..., None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
    a = Q[np.arange(n)[None, :], O]; dist = np.zeros((NS, n + 1)); dist[:, 0] = 1
    for i in range(n):
        x = a[:, i:i + 1]; dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    good = (T[None, :, :] == O[:, None, :]).sum(axis=2)                  # NS × 16
    out = np.zeros((NS, PER_GRID + 1))
    for r in range(len(M)):
        lam = S * dist[:, n - r]
        mu = prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
        mu = np.where(lam < 3, mu * low_mult, mu)
        W = rng.poisson(rng.gamma(TH, mu / TH))[:, None]
        c = np.cumsum(good == n - r, axis=1)                             # nos gagnants parmi les k premiers
        out[:, 1:] += np.where(c > 0, M[r] * c / (W + c), 0.0)
    return out


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    T = B16.greedy(P, Q, S, M, prm, PER_GRID, 1)
    Pt = (1 - SHRINK) * P + SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    base = [lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"])[0] for t in T]
    s3 = [lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=LOW_MULT)[0] for t in T]
    order = np.argsort(s3, kind="stable")[::-1]
    T = np.array([T[k] for k in order])
    rng = np.random.default_rng([g["game"], g["gid"]])                   # graine propre à la grille : résultat reproductible
    return {"game": g["game"], "train": g["close"] < "2024", "s3": [s3[k] for k in order], "base": [base[k] for k in order],
            "sim_s3": prefix_gains(g, T, Pt, Q, rng, LOW_MULT).astype(np.float32),
            "sim_base": prefix_gains(g, T, P, Q, rng, 1.0).astype(np.float32)}


def load_or_build():
    if os.path.exists(CACHE):
        z = np.load(CACHE)
        return {k: z[k] for k in z.files}
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    with Pool(8) as pool:
        ev = pool.map(work, grids, chunksize=4)
    data = {"game": np.array([e["game"] for e in ev]), "train": np.array([e["train"] for e in ev]),
            "s3": np.array([e["s3"] for e in ev]), "base": np.array([e["base"] for e in ev]),
            "sim_s3": np.stack([e["sim_s3"] for e in ev]), "sim_base": np.stack([e["sim_base"] for e in ev])}
    np.savez_compressed(CACHE, **data)
    return data


def arrivals(data, mask, seed):
    """Arrivées communes à toutes les configurations : (YEARS·12) mois × A emplacements ; -1 = emplacement vide."""
    rng = np.random.default_rng(seed)
    idx = {k: np.flatnonzero(mask & (data["game"] == k)) for k in (7, 8)}
    months = YEARS * 12
    counts = {k: rng.poisson(LAM[k], months) for k in (7, 8)}
    width = int((counts[7] + counts[8]).max())
    G = np.full((months, width), -1, dtype=np.int64)
    for m in range(months):
        row = np.concatenate([rng.choice(idx[7], counts[7][m]), rng.choice(idx[8], counts[8][m])])
        rng.shuffle(row); G[m, :len(row)] = row
    D = rng.integers(0, NS, G.shape)                                     # tirage utilisé pour chaque grille arrivée
    return G, D


def evaluate(data, G, D, tau, budget):
    valid = G >= 0; gi = np.where(valid, G, 0)
    demand = np.where(valid, (data["s3"] >= tau).sum(axis=1)[gi], 0)     # les tickets sont classés par S3 décroissante
    before = np.cumsum(demand, axis=1) - demand
    k = np.clip(budget - before, 0, demand)                              # tickets joués sur chaque grille (règle séquentielle)
    cum_s3 = np.concatenate([np.zeros((len(data["s3"]), 1)), np.cumsum(data["s3"] - 1, axis=1)], axis=1)
    cum_base = np.concatenate([np.zeros((len(data["base"]), 1)), np.cumsum(data["base"] - 1, axis=1)], axis=1)
    ev_s3 = np.where(valid, cum_s3[gi, k], 0.0); ev_base = np.where(valid, cum_base[gi, k], 0.0)
    net_s3 = np.where(valid, data["sim_s3"][gi, D, k], 0.0) - k          # résultat net simulé de chaque grille
    spend = k.sum(axis=1)
    per_grid = net_s3.reshape(YEARS, -1)                                 # années × (12 mois × A), dans l'ordre d'arrivée
    run = np.cumsum(per_grid, axis=1)
    drawdown = (np.maximum.accumulate(np.maximum(run, 0), axis=1) - run).max(axis=1)
    year = run[:, -1]
    return {"tau": tau, "budget": budget, "spend_month": float(spend.mean()), "capped": float(np.mean(spend == budget)),
            "ev_s3_year": float(ev_s3.sum() / YEARS), "ev_base_year": float(ev_base.sum() / YEARS),
            "sim_s3_year": float(year.mean()), "p_loss": float(np.mean(year < 0)), "median": float(np.median(year)),
            "p10": float(np.percentile(year, 10)), "p90": float(np.percentile(year, 90)),
            "dd_median": float(np.median(drawdown)), "dd_p90": float(np.percentile(drawdown, 90))}


def sweep(data, mask, seed):
    G, D = arrivals(data, mask, seed)
    return [evaluate(data, G, D, t, b) for b in BUDGETS for t in TAUS]


def best_by_budget(rows):
    return {b: max((r for r in rows if r["budget"] == b), key=lambda r: r["ev_s3_year"]) for b in BUDGETS}


def show(best, title):
    print(f"\n{title}")
    print("budget  τ*   dépense/mois  au plafond  EV S3/an  EV base/an  simulé S3/an  P(perte/an)  médiane    p10   repli médian  repli p90")
    for b, r in best.items():
        print(f"{b:4d} €  {r['tau']:.1f}  {r['spend_month']:7.1f} €    {r['capped']:5.0%}   {r['ev_s3_year']:+7.0f} €  {r['ev_base_year']:+7.0f} €"
              f"   {r['sim_s3_year']:+7.0f} €     {r['p_loss']:5.1%}    {r['median']:+6.0f} € {r['p10']:+6.0f} €   {r['dd_median']:6.0f} €  {r['dd_p90']:6.0f} €")


if __name__ == "__main__":
    data = load_or_build()
    ref = json.load(open("../data/lf/weekly_budget_ev.json"))
    gap = max(abs(a - b) for e, r in zip(data["s3"], ref) for a, b in zip(e, r["s3"]))
    print(f"contrôle : écart maximal des espérances S3 avec l'étude du §47 = {gap:.2e}")
    train, test = data["train"].astype(bool), ~data["train"].astype(bool)
    print(f"grilles : {train.sum()} d'entraînement, {test.sum()} de test ; configurations : {len(TAUS)} τ × {len(BUDGETS)} budgets")
    rows_train = sweep(data, train, seed=1)
    best = best_by_budget(rows_train)
    show(best, "Entraînement : τ* par budget (critère : EV S3 par an)")
    if "--test" in sys.argv:
        rows_test = sweep(data, test, seed=2)
        show({b: next(r for r in rows_test if r["budget"] == b and r["tau"] == best[b]["tau"]) for b in BUDGETS},
             "Test, au τ* choisi sur l'entraînement")
        show(best_by_budget(rows_test), "Test, τ* propre au test (pour information, ne sert pas au choix)")
    else:
        rows_test = None
    json.dump({"taus": TAUS, "budgets": BUDGETS, "train": rows_train, "test": rows_test,
               "tau_star": {str(b): best[b]["tau"] for b in BUDGETS}}, open(OUT, "w"))
