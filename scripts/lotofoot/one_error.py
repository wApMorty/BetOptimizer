"""Écarter les tickets dont la part « une erreur » rend moins que la mise ? (question de Paul du 2026-10-25)

Constat de Paul : sur la page `/lotofoot`, certains tickets ont, au rang « une erreur », une chance et un gain dont le produit est
inférieur à 1 € (LF7 n°133, ticket 1222221 : 1 sur 59 pour ≈ 56 €, soit 0,95 €). Question : que donnerait un système qui ne les
joue pas ?

Protocole, figé avant le calcul (calculs ex ante : espérances et tirages du modèle gelé, aucun résultat de match) :
- grilles et tickets : ceux de l'abaque (`budget_tau.py`, §49) : 409 grilles LF7/LF8 de l'étude FDJ, 16 tickets de la recherche de
  production par grille (cache de `one_error_peek.py`, contrôlé contre le cache de l'abaque) ;
- définition : part « une erreur » d'un ticket = P(une erreur) · gain moyen sachant une erreur = E[gain · 1{une erreur}], dans le
  modèle de **base**, celui qu'affiche la page ; ticket « faible » si cette part est < 1 € (prix du ticket) ;
- règles comparées : A, production (tickets d'espérance S3 ≥ τ, les meilleurs d'abord) ; B, la même sans les tickets faibles (le
  budget libéré va aux tickets suivants, sur la même grille ou les suivantes) ;
- simulation : celle de l'abaque, inchangée (mois au rythme 2025-2026, ordre d'arrivée aléatoire, plafond mensuel, 16 tickets au
  plus par grille, 2 000 tirages par grille, partage entre nos tickets compris, 4 000 années) ; mêmes arrivées et mêmes tirages des
  issues et des concurrents pour A et B (comparaison appariée) ;
- configurations : budgets de 20 à 400 € par pas de 20 € ; A au τ* de l'abaque ; B au même τ, puis B à son propre τ* (31 valeurs
  de τ, critère de l'abaque : espérance S3 par an, maximale sur l'entraînement), soit 20 + 20 + 620 configurations ;
- lecture : écart B − A de l'espérance S3 par an, et du risque (probabilité d'une année en perte, médiane, 10e centile, repli) ;
  entraînement (grilles closes avant 2024) pour le choix de τ*, test (2024 et après) lu ensuite.
Attendu sous le modèle, avant calcul : sans contrainte de budget, retirer un ticket d'espérance ≥ τ > 1 ne peut que baisser
l'espérance ; avec le plafond, l'effet dépend de ce qui remplace le ticket retiré.
Limites : celles de l'abaque (répartition finale, coupes et sélections absentes, mois non indépendants, aucun gain démontré, §37).

Usage, depuis scripts/lotofoot : `python3 one_error.py [--test]` (journal horodaté sur stderr).
"""
import sys, json, os, time, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import budget_tau as BT
import backtest_fdj as BF
from frontier import PRM

PRICE = 1.0
CACHE = "../data/lf/one_error_cache.npz"
OUT = "../data/lf/one_error.json"
T0 = time.time()


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')} +{time.time() - T0:6.0f} s] {msg}", file=sys.stderr, flush=True)


def work(args):
    """Tickets d'une grille dans l'ordre de la règle B (non faibles par S3 décroissante, puis faibles) et gains simulés de ses
    préfixes. La graine est celle de l'abaque : issues et concurrents identiques à ceux de la règle A."""
    g, tickets = args
    P, Q = np.array(g["p"]), np.array(g["q"])
    Pt = (1 - BT.SHRINK) * P + BT.SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    weak = [t["c_base"][1] < PRICE for t in tickets]
    order = [k for k in range(len(tickets)) if not weak[k]] + [k for k in range(len(tickets)) if weak[k]]
    T = np.array([tickets[k]["t"] for k in order])
    rng = np.random.default_rng([g["game"], g["gid"]])
    sim_s3 = BT.prefix_gains(g, T, Pt, Q, rng, BT.LOW_MULT).astype(np.float32)
    # Les tickets faibles ne sont jamais joués par B : espérance mise très bas pour que la demande ne les compte pas.
    s3 = [tickets[k]["s3"] if not weak[k] else -1e9 for k in order]
    base = [tickets[k]["base"] for k in order]
    return {"s3_b": s3, "base_b": base, "sim_s3_b": sim_s3, "weak": [weak[k] for k in order]}


def build():
    if os.path.exists(CACHE):
        log(f"données lues dans le cache {CACHE}")
        z = np.load(CACHE)
        return {k: z[k] for k in z.files}
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    peek = {(p["game"], p["gid"]): p["tickets"] for p in json.load(open("../data/lf/one_error_tickets.json"))}
    import one_error_peek as OP
    log(f"décomposition par rang en base des {16 * len(grids)} tickets")
    jobs = []
    for g in grids:
        P, Q = np.array(g["p"]), np.array(g["q"]); M = BF.masses(g); prm = PRM[g["game"]]
        tk = peek[(g["game"], g["gid"])]
        for t in tk:
            t["c_base"] = [c for _, c in OP.ev_ranks(t["t"], P, Q, g["S"], M, prm["kappa"], prm["beta"], prm["phi"])]
        jobs.append((g, tk))
    log("tirages de la règle B (8 processus)")
    out = []
    with Pool(8) as pool:
        for k, r in enumerate(pool.imap(work, jobs, chunksize=2), 1):
            out.append(r)
            if k % 40 == 0 or k == len(jobs):
                el = time.time() - T0
                log(f"  {k} / {len(jobs)} grilles, reste ≈ {el / k * (len(jobs) - k):.0f} s")
    a = BT.load_or_build()
    gap = max(abs(x - y) for e, (g, tk) in zip(a["s3"], jobs) for x, y in zip(e, [t["s3"] for t in tk]))
    log(f"contrôle : écart maximal des espérances S3 avec le cache de l'abaque = {gap:.2e}")
    data = {"game": a["game"], "train": a["train"], "s3": a["s3"], "base": a["base"], "sim_s3": a["sim_s3"],
            "s3_b": np.array([r["s3_b"] for r in out]), "base_b": np.array([r["base_b"] for r in out]),
            "sim_s3_b": np.stack([r["sim_s3_b"] for r in out]), "weak": np.array([r["weak"] for r in out]),
            "s3_sorted_b": np.array([[t["s3"] for t in sorted(tk, key=lambda t: t["c_base"][1] < PRICE)] for g, tk in jobs])}
    np.savez_compressed(CACHE, **data)
    return data


def rule_b(data):
    return {"game": data["game"], "s3": data["s3_b"], "base": data["base_b"], "sim_s3": data["sim_s3_b"]}


def describe(data, tau_by_budget):
    """Part des tickets ≥ τ qui sont faibles, et leur espérance, au τ de quelques budgets."""
    for b in (20, 60, 100, 200, 400):
        tau = tau_by_budget[b]
        s3 = data["s3_sorted_b"]; weak = data["weak"]
        for game in (7, 8):
            m = (data["game"] == game)[:, None] & (s3 >= tau)
            w = m & weak
            print(f"  budget {b:3d} € (τ = {tau}), LF{game} : {m.sum():5d} tickets ≥ τ, dont {w.sum():4d} faibles "
                  f"({w.sum() / max(m.sum(), 1):.0%}) ; espérance S3 moyenne des faibles {s3[w].mean() if w.any() else float('nan'):.2f}, "
                  f"des autres {s3[m & ~weak].mean():.2f}")


def table(rows_a, rows_b, rows_b_own, title):
    print(f"\n{title}")
    print("budget  τ    | A : EV S3/an  P(perte)  médiane  p10   | B même τ : ΔEV/an  P(perte)  médiane  p10   dépense | "
          "B τ* propre : τ   ΔEV/an")
    for b in BT.BUDGETS:
        a, x, y = rows_a[b], rows_b[b], rows_b_own[b]
        print(f"{b:4d} €  {a['tau']:.1f}  | {a['ev_s3_year']:+7.0f} €   {a['p_loss']:5.1%}  {a['median']:+6.0f} € {a['p10']:+6.0f} € | "
              f"{x['ev_s3_year'] - a['ev_s3_year']:+6.0f} €   {x['p_loss']:5.1%}  {x['median']:+6.0f} € {x['p10']:+6.0f} € "
              f"{x['spend_month'] - a['spend_month']:+5.1f} € | {y['tau']:.1f}  {y['ev_s3_year'] - a['ev_s3_year']:+6.0f} €")


def run(data, mask, seed, tau_star, label):
    log(f"{label} : arrivées communes ({BT.YEARS} années)")
    G, D = BT.arrivals(data, mask, seed)
    b_data = rule_b(data)
    rows_a, rows_b, all_b = {}, {}, []
    for i, b in enumerate(BT.BUDGETS, 1):
        rows_a[b] = BT.evaluate(data, G, D, tau_star[b], b)
        rows_b[b] = BT.evaluate(b_data, G, D, tau_star[b], b)
        all_b += [BT.evaluate(b_data, G, D, t, b) for t in BT.TAUS]
        log(f"  {label} : budget {i} / {len(BT.BUDGETS)} ({b} €)")
    return rows_a, rows_b, all_b


if __name__ == "__main__":
    data = build()
    tau_star = {int(k): v for k, v in json.load(open("../data/lf/budget_tau.json"))["tau_star"].items()}
    train, test = data["train"].astype(bool), ~data["train"].astype(bool)
    print("Tickets faibles (part « une erreur » < 1 € en base) parmi les tickets au-dessus du τ de l'abaque, 409 grilles :")
    describe(data, tau_star)
    rows_a, rows_b, all_b = run(data, train, 1, tau_star, "entraînement")
    own = BT.best_by_budget(all_b)
    table(rows_a, rows_b, own, "Entraînement (319 grilles) : A = production, B = sans les tickets faibles")
    result = {"train": {"a": rows_a, "b": rows_b, "b_own": own}}
    if "--test" in sys.argv:
        ta, tb, tall = run(data, test, 2, tau_star, "test")
        tb_own = {b: next(r for r in tall if r["budget"] == b and r["tau"] == own[b]["tau"]) for b in BT.BUDGETS}
        table(ta, tb, tb_own, "Test (90 grilles), τ choisis sur l'entraînement")
        result["test"] = {"a": ta, "b": tb, "b_own": tb_own}
    json.dump({k: {r: {str(b): v for b, v in d.items()} for r, d in s.items()} for k, s in result.items()}, open(OUT, "w"))
    log(f"résultats écrits dans {OUT}")
