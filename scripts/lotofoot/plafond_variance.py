"""Relever le plafond de 16 tickets par grille réduit-il la variance ? (demande de Paul du 2026-10-30)

Hypothèse de Paul : plus de tickets par grille, donc des gains plus fréquents, donc moins de variance et plus de gains recyclés.
Protocole, figé avant le calcul :
- grilles : les 409 grilles LF7/LF8 du §47 ; tickets : vivier de 64 candidats classés par espérance S3 (§63) ;
- trois lots : entraînement (avant 2024), test (2024 et après), et « riches » : les grilles d'au moins 27 tickets ≥ 1,7, comme les cinq
  grilles du journal d'octobre 2026 (27 à 54), sans distinction de date ;
- tirages : 2 000 par grille, scénario S3, comme `abaque_vivier.py` (partage entre nos tickets compris) ; scénario « public juste » en
  lecture, à τ = 1,7 ;
- règle : celle de production, budget mensuel avec gains recyclés (§67) ; seul le plafond par grille change ;
- configurations : 7 plafonds (8, 12, 16, 24, 32, 48, 64) × 2 budgets (100, 200 €) × 6 τ (1,5 à 2,0), soit 84 par lot ; les plafonds 8 et 12
  ont été ajoutés après lecture de la première passe (`plafond_variance_passe1.txt`), qui ne portait que sur 16 et au-delà ;
- mesures : écart type et centiles du résultat annuel, année en perte, grilles payées par an, années et mois avec un gain, mises.
  L'écart type est dominé par les rares sans-faute : les centiles se lisent à côté. Aucun critère de choix : étude descriptive.
Limites : mises S connues ; répartition finale ; lots tirés avec remise ; le lot « riches » est petit et presque entièrement LF8 ;
aucun gain démontré (le test figé du §37 a échoué).

Usage, depuis scripts/lotofoot, en arrière-plan :
  ~/.local/share/edge/etudes/venv/bin/python plafond_variance.py > ../data/lf/plafond_variance.txt 2> ../data/lf/plafond_variance.log &
"""
import sys, json, time, datetime as dt, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM
import budget_tau as BT
import abaque_vivier as AV

KEEP, POOL = 64, 64
AV.KEEP = KEEP                     # `draws` calcule les gains des k premiers tickets, k = 0…KEEP
YEARS, LAG = BT.YEARS, AV.LAG
CAPS = [8, 12, 16, 24, 32, 48, 64]
BUDGETS = [100, 200]
TAUS = [1.5, 1.6, 1.7, 1.8, 1.9, 2.0]
TAU_PROD, RICH = 1.7, 27
OUT = "../data/lf/plafond_variance.json"
START = time.time()


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - START:6.0f} s] {message}", file=sys.stderr, flush=True)


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    T = B16.greedy(P, Q, S, M, prm, POOL, 1)
    Pt = (1 - BT.SHRINK) * P + BT.SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    s3 = [lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=BT.LOW_MULT)[0] for t in T]
    order = sorted(range(POOL), key=lambda k: -s3[k])
    T = np.array([T[k] for k in order])
    rng = np.random.default_rng([g["game"], g["gid"], 64])
    sim_s3, _ = AV.draws(g, T, Pt, Q, rng, BT.LOW_MULT)
    sim_null, _ = AV.draws(g, T, Q, Q, rng, 1.0)
    return {"game": g["game"], "train": g["close"] < "2024", "s3": [s3[k] for k in order],
            "sim_s3": sim_s3.astype(np.float32), "sim_null": sim_null.astype(np.float32)}


def run(data, G, D, tau, budget, cap, scenario):
    sim = data[f"sim_{scenario}"]
    valid = G >= 0; gi = np.where(valid, G, 0)
    demand = np.minimum(np.where(valid, (data["s3"] >= tau).sum(axis=1)[gi], 0), cap)
    months, width = G.shape
    left = np.full(months, float(budget))
    K = np.zeros((months, width), dtype=np.int64); gains = np.zeros((months, width))
    for a in range(width):
        if a >= LAG:
            left += gains[:, a - LAG]
        k = np.minimum(demand[:, a], np.floor(np.maximum(left, 0)).astype(np.int64))
        K[:, a] = k
        gains[:, a] = sim[gi[:, a], D[:, a], k]
        left -= k
    return K, gains


def summary(K, gains, tau, budget, cap, label):
    net = (gains - K).reshape(YEARS, -1)
    year = net.sum(axis=1)
    paid = gains > 0
    return {"lot": label, "tau": tau, "budget": budget, "cap": cap, "stakes_month": float(K.sum(axis=1).mean()),
            "grids_month": float((K > 0).sum(axis=1).mean()), "tickets_grid": float(K.sum() / max((K > 0).sum(), 1)),
            "mean": float(year.mean()), "sd": float(year.std()), "p10": float(np.percentile(year, 10)), "p25": float(np.percentile(year, 25)),
            "median": float(np.median(year)), "p75": float(np.percentile(year, 75)), "p90": float(np.percentile(year, 90)),
            "p_loss": float(np.mean(year < 0)), "paid_year": float(paid.sum() / YEARS),
            "p_win_year": float(np.mean(paid.reshape(YEARS, -1).any(axis=1))), "p_win_month": float(np.mean(paid.any(axis=1))),
            "gains_month_median": float(np.median(gains.sum(axis=1)))}


def show(rows, title):
    print(f"\n{title}")
    print("budget   τ  plafond  mises/mois  grilles/mois  tickets/grille  moyenne/an  écart type     p10     p25  médiane     p75     p90  P(perte)  grilles payées/an  P(gain/mois)  P(gain/an)")
    for r in rows:
        print(f"{r['budget']:4d} €  {r['tau']:.1f}   {r['cap']:3d}    {r['stakes_month']:6.1f} €     {r['grids_month']:5.1f}         {r['tickets_grid']:5.1f}     {r['mean']:+7.0f} €  {r['sd']:7.0f} €  {r['p10']:+6.0f}  {r['p25']:+6.0f}  {r['median']:+6.0f}  {r['p75']:+6.0f}  {r['p90']:+6.0f}   {r['p_loss']:5.1%}      {r['paid_year']:6.2f}          {r['p_win_month']:5.1%}      {r['p_win_year']:5.1%}")


def best_rows(rows, budget):
    """Pour chaque plafond, le τ de plus grande moyenne : la comparaison la plus favorable à chaque plafond."""
    out = []
    for cap in CAPS:
        out.append(max((r for r in rows if r["budget"] == budget and r["cap"] == cap), key=lambda r: r["mean"]))
    return out


if __name__ == "__main__":
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    log(f"{len(grids)} grilles, {POOL} candidats, {BT.NS} tirages par grille, deux scénarios")
    ev = []
    with Pool(8) as pool:
        for k, e in enumerate(pool.imap(work, grids, chunksize=2), 1):
            ev.append(e)
            if k % 40 == 0 or k == len(grids):
                log(f"grilles {k} / {len(grids)}, reste environ {(time.time() - START) / k * (len(grids) - k):.0f} s")
    data = {k: np.array([e[k] for e in ev]) for k in ("game", "train", "s3")}
    for k in ("sim_s3", "sim_null"):
        data[k] = np.stack([e[k] for e in ev])
    del ev
    data["game"] = data["game"].astype(np.int64)
    train = data["train"].astype(bool)
    rich = (data["s3"] >= TAU_PROD).sum(axis=1) >= RICH
    log(f"lot riche : {rich.sum()} grilles, dont {(rich & (data['game'] == 7)).sum()} LF7")
    print(f"lot riche : {rich.sum()} grilles, dont {(rich & (data['game'] == 7)).sum()} LF7")
    results, total, done = [], 3 * len(CAPS) * len(BUDGETS) * len(TAUS), 0
    begin = time.time()
    for label, mask, seed in (("entraînement", train, 1), ("test", ~train, 2), ("riches", rich, 3)):
        G, D = BT.arrivals(data, mask, seed)
        rows = []
        for budget in BUDGETS:
            for cap in CAPS:
                for tau in TAUS:
                    K, gains = run(data, G, D, tau, budget, cap, "s3")
                    rows.append(summary(K, gains, tau, budget, cap, label))
                    done += 1
                log(f"{done} / {total} configurations, reste environ {(time.time() - begin) / done * (total - done):.0f} s")
        results += rows
        for budget in BUDGETS:
            show([r for r in rows if r["budget"] == budget and r["tau"] == TAU_PROD], f"{label}, {budget} € par mois, τ = {TAU_PROD}")
            show(best_rows(rows, budget), f"{label}, {budget} € par mois, meilleur τ de chaque plafond")
        null = []
        for budget in BUDGETS:
            for cap in CAPS:
                K, gains = run(data, G, D, TAU_PROD, budget, cap, "null")
                null.append(summary(K, gains, TAU_PROD, budget, cap, label + " public juste"))
        show(null, f"{label}, scénario « public juste », τ = {TAU_PROD}")
        results += null
    json.dump(results, open(OUT, "w"))
    log(f"fini, résultats dans {OUT}")
