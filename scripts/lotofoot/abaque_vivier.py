"""Abaque du seuil τ avec le vivier de `lotofoot-2`, et budget compté net des gains recyclés (demande de Paul du 2026-10-30).

Protocole, figé avant le calcul :
- grilles : les 409 grilles LF7/LF8 du §47 ; tickets : vivier de 64 candidats (§63), classés par espérance S3, 16 par grille au plus ;
- tirages : 2 000 par grille, comme `budget_tau.py` (issues, gagnants concurrents en binomiale négative, partage entre nos tickets),
  dans trois scénarios : base, S3 (p rapproché de q de 10 %, concurrents × 1,4 si λ < 3) et « public juste » (les issues suivent la
  répartition du public : les biais mesurés n'existent pas ; les tickets restent ceux choisis avec les cotes) ;
- mois simulés : ceux de `budget_tau.py` (rythmes 2025-2026, ordre aléatoire, 4 000 années, arrivées communes) ;
- deux modes de budget :
  « brut » (production) : les mises du mois ne dépassent pas le budget ;
  « recyclé » : les mises du mois moins les gains déjà encaissés ne dépassent pas le budget. Un gain s'ajoute au disponible deux grilles
  après celle qui le produit (LAG, environ deux jours au rythme de 6,6 grilles par semaine). Le disponible repart de zéro chaque mois ;
- configurations : 31 τ × 20 budgets × 2 modes, soit 1 240 ;
- critère : τ* maximise l'espérance S3 par an (Σ (EV S3 − 1) des tickets joués sur les trajectoires S3), sur l'entraînement ; le test
  (2024 et après) est lu ensuite. Le gain simulé, le risque et le scénario « public juste » s'affichent sans servir au choix.
Limites : mises S connues (l'erreur d'estimation du §64 s'ajoute) ; répartition finale ; grilles de championnat seulement ; mois réels non
indépendants ; aucun gain démontré (le test figé du §37 a échoué) ; le scénario « public juste » garde le pactole et n'est pas un plancher.

Usage, depuis scripts/lotofoot, en arrière-plan :
  ~/.local/share/edge/etudes/venv/bin/python abaque_vivier.py > ../data/lf/abaque_vivier.txt 2> ../data/lf/abaque_vivier.log &
Progression sur stderr ; tirages en cache dans ../data/lf/abaque_vivier_cache.npz, résultats dans ../data/lf/abaque_vivier.json.
"""
import sys, json, os, time, datetime as dt, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM
import budget_tau as BT

KEEP, POOL, LAG = 16, 64, 2
NS, YEARS, TH = BT.NS, BT.YEARS, BT.TH
TAUS, BUDGETS = BT.TAUS, BT.BUDGETS
MODES = ["brut", "recyclé"]
CACHE = "../data/lf/abaque_vivier_cache.npz"
OUT = "../data/lf/abaque_vivier.json"
START = time.time()


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - START:6.0f} s] {message}", file=sys.stderr, flush=True)


def draws(g, T, P, Q, rng, low_mult):
    """Gain brut des k premiers tickets (k = 0…16) et présence d'un sans-faute parmi eux, sur NS tirages (voir `budget_tau.prefix_gains`)."""
    n = g["n"]; S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    O = (rng.random((NS, n))[..., None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
    a = Q[np.arange(n)[None, :], O]; dist = np.zeros((NS, n + 1)); dist[:, 0] = 1
    for i in range(n):
        x = a[:, i:i + 1]; dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    good = (T[None, :, :] == O[:, None, :]).sum(axis=2)
    out = np.zeros((NS, KEEP + 1)); perfect = np.zeros((NS, KEEP + 1), dtype=bool)
    for r in range(len(M)):
        lam = S * dist[:, n - r]
        mu = prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
        mu = np.where(lam < 3, mu * low_mult, mu)
        W = rng.poisson(rng.gamma(TH, mu / TH))[:, None]
        c = np.cumsum(good == n - r, axis=1)
        out[:, 1:] += np.where(c > 0, M[r] * c / (W + c), 0.0)
        if r == 0:
            perfect[:, 1:] = c > 0
    return out, perfect


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    T = B16.greedy(P, Q, S, M, prm, POOL, 1)
    Pt = (1 - BT.SHRINK) * P + BT.SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    base = [lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"])[0] for t in T]
    s3 = [lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=BT.LOW_MULT)[0] for t in T]
    order = sorted(range(POOL), key=lambda k: -s3[k])[:KEEP]
    T = np.array([T[k] for k in order])
    rng = np.random.default_rng([g["game"], g["gid"]])
    sim_s3, per_s3 = draws(g, T, Pt, Q, rng, BT.LOW_MULT)
    sim_base, per_base = draws(g, T, P, Q, rng, 1.0)
    sim_null, per_null = draws(g, T, Q, Q, rng, 1.0)
    return {"game": g["game"], "train": g["close"] < "2024", "s3": [s3[k] for k in order], "base": [base[k] for k in order],
            "sim_s3": sim_s3.astype(np.float32), "sim_base": sim_base.astype(np.float32), "sim_null": sim_null.astype(np.float32),
            "per_s3": per_s3, "per_base": per_base, "per_null": per_null}


def load_or_build():
    if os.path.exists(CACHE):
        z = np.load(CACHE)
        log("tirages lus dans le cache")
        return {k: z[k] for k in z.files}
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    log(f"{len(grids)} grilles, {POOL} candidats, {NS} tirages par grille et par scénario")
    ev = []
    with Pool(8) as pool:
        for k, e in enumerate(pool.imap(work, grids, chunksize=2), 1):
            ev.append(e)
            if k % 20 == 0 or k == len(grids):
                log(f"grilles {k} / {len(grids)}, reste environ {(time.time() - START) / k * (len(grids) - k):.0f} s")
    data = {k: np.array([e[k] for e in ev]) for k in ("game", "train", "s3", "base")}
    for k in ("sim_s3", "sim_base", "sim_null", "per_s3", "per_base", "per_null"):
        data[k] = np.stack([e[k] for e in ev])
    np.savez_compressed(CACHE, **data)
    return data


def run(data, G, D, tau, budget, mode, scenario):
    """Déroule les mois grille par grille. Le disponible dépend des gains : le calcul avance donc emplacement par emplacement."""
    sim, perfect = data[f"sim_{scenario}"], data[f"per_{scenario}"]
    valid = G >= 0; gi = np.where(valid, G, 0)
    demand = np.where(valid, (data["s3"] >= tau).sum(axis=1)[gi], 0)
    months, width = G.shape
    left = np.full(months, float(budget))
    K = np.zeros((months, width), dtype=np.int64)
    gains = np.zeros((months, width)); wins = np.zeros((months, width), dtype=bool)
    for a in range(width):
        if mode == 1 and a >= LAG:
            left += gains[:, a - LAG]
        k = np.minimum(demand[:, a], np.floor(np.maximum(left, 0)).astype(np.int64))
        K[:, a] = k
        gains[:, a] = sim[gi[:, a], D[:, a], k]
        wins[:, a] = perfect[gi[:, a], D[:, a], k]
        left -= k
    return K, gains, wins


def summary(data, G, D, K, gains, wins, tau, budget, mode):
    valid = G >= 0; gi = np.where(valid, G, 0)
    cum_s3 = np.concatenate([np.zeros((len(data["s3"]), 1)), np.cumsum(data["s3"] - 1, axis=1)], axis=1)
    cum_base = np.concatenate([np.zeros((len(data["base"]), 1)), np.cumsum(data["base"] - 1, axis=1)], axis=1)
    net = gains - K
    run_ = np.cumsum(net.reshape(YEARS, -1), axis=1)
    drawdown = (np.maximum.accumulate(np.maximum(run_, 0), axis=1) - run_).max(axis=1)
    year = run_[:, -1]
    outlay = (K.sum(axis=1) - gains.sum(axis=1))
    return {"tau": tau, "budget": budget, "mode": MODES[mode], "stakes_month": float(K.sum(axis=1).mean()),
            "stakes_median": float(np.median(K.sum(axis=1))), "outlay_median": float(np.median(outlay)),
            "ev_s3_year": float(np.where(valid, cum_s3[gi, K], 0.0).sum() / YEARS),
            "ev_base_year": float(np.where(valid, cum_base[gi, K], 0.0).sum() / YEARS),
            "sim_year": float(year.mean()), "p_loss": float(np.mean(year < 0)), "median": float(np.median(year)),
            "p10": float(np.percentile(year, 10)), "p90": float(np.percentile(year, 90)), "dd_p90": float(np.percentile(drawdown, 90)),
            "p_win_year": float(np.mean((gains > 0).reshape(YEARS, -1).any(axis=1))),
            "p_perfect_year": float(np.mean(wins.reshape(YEARS, -1).any(axis=1)))}


def sweep(data, G, D, label):
    rows, total, begin = [], len(TAUS) * len(BUDGETS) * len(MODES), time.time()
    for mode in range(2):
        for b, budget in enumerate(BUDGETS):
            for tau in TAUS:
                K, gains, wins = run(data, G, D, tau, budget, mode, "s3")
                rows.append(summary(data, G, D, K, gains, wins, tau, budget, mode))
            done = len(rows)
            log(f"{label} : {done} / {total} configurations, reste environ {(time.time() - begin) / done * (total - done):.0f} s")
    return rows


def best_by(rows):
    return {(r["mode"], r["budget"]): r for r in sorted(rows, key=lambda r: r["ev_s3_year"])}


def find(rows, mode, budget, tau):
    return next(r for r in rows if r["mode"] == mode and r["budget"] == budget and r["tau"] == tau)


def show(best, title):
    print(f"\n{title}")
    print("mode     budget  τ    mises/mois  mises méd.  dépense nette méd.  EV S3/an  EV base/an  simulé S3/an  P(perte/an)  médiane    p10   repli p90  P(≥1 gain/an)  P(sans-faute/an)")
    for r in best:
        print(f"{r['mode']:8s} {r['budget']:4d} €  {r['tau']:.1f}  {r['stakes_month']:7.1f} €  {r['stakes_median']:6.0f} €  {r['outlay_median']:8.1f} €       {r['ev_s3_year']:+7.0f} €  {r['ev_base_year']:+7.0f} €"
              f"   {r['sim_year']:+7.0f} €     {r['p_loss']:5.1%}    {r['median']:+6.0f} € {r['p10']:+6.0f} €  {r['dd_p90']:6.0f} €    {r['p_win_year']:6.1%}        {r['p_perfect_year']:6.1%}")


def compare_abaque(rows, best, label):
    current = json.load(open("../data/lf/budget_tau.json"))["tau_star"]
    print(f"\n{label} : abaque actuel contre τ* du vivier, budget brut (espérance S3 par an)")
    print("budget  τ actuel  EV S3   τ vivier  EV S3   écart")
    for budget in BUDGETS:
        now = find(rows, "brut", budget, current[str(budget)])
        star = best[("brut", budget)]
        print(f"{budget:4d} €   {now['tau']:.1f}    {now['ev_s3_year']:+7.0f} €   {star['tau']:.1f}    {star['ev_s3_year']:+7.0f} €   {star['ev_s3_year'] - now['ev_s3_year']:+5.0f} €")


if __name__ == "__main__":
    data = load_or_build()
    data["game"] = data["game"].astype(np.int64)
    train, test = data["train"].astype(bool), ~data["train"].astype(bool)
    log(f"grilles : {train.sum()} d'entraînement, {test.sum()} de test")
    G_train, D_train = BT.arrivals(data, train, 1)
    G_test, D_test = BT.arrivals(data, test, 2)
    log("arrivées tirées")
    rows_train = sweep(data, G_train, D_train, "entraînement")
    best = best_by(rows_train)
    show([best[(m, b)] for m in MODES for b in BUDGETS], "Entraînement : τ* (critère : espérance S3 par an)")
    compare_abaque(rows_train, best, "Entraînement")
    rows_test = sweep(data, G_test, D_test, "test")
    show([find(rows_test, m, b, best[(m, b)]["tau"]) for m in MODES for b in BUDGETS], "Test, au τ* de l'entraînement")
    compare_abaque(rows_test, {k: find(rows_test, k[0], k[1], best[k]["tau"]) for k in best}, "Test (τ* de l'entraînement)")
    print("\nScénario « public juste », τ* de l'entraînement, grilles de test puis d'entraînement :")
    print("mode     budget  τ    grilles   mises/mois  résultat moyen/an  médiane/an  P(perte/an)  P(sans-faute/an)")
    null = []
    for G, D, label in ((G_test, D_test, "test"), (G_train, D_train, "entr.")):
        for m, name in enumerate(MODES):
            for budget in (60, 100, 200, 400):
                r = best[(name, budget)]
                K, gains, wins = run(data, G, D, r["tau"], budget, m, "null")
                s = summary(data, G, D, K, gains, wins, r["tau"], budget, m)
                null.append({**s, "split": label})
                print(f"{name:8s} {budget:4d} €  {r['tau']:.1f}  {label:6s}   {s['stakes_month']:7.1f} €   {s['sim_year']:+8.0f} €     {s['median']:+7.0f} €    {s['p_loss']:5.1%}        {s['p_perfect_year']:6.1%}")
    json.dump({"train": rows_train, "test": rows_test, "null": null}, open(OUT, "w"))
    log(f"fini, résultats dans {OUT}")
