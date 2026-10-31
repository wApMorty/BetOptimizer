"""Modèle d'optimisation du revenu net du Loto Foot : le choix des tickets posé en sac à dos en ligne (demande de Paul du 2026-10-31).

Le problème. Chaque mois, des grilles arrivent une à une ; on engage au plus B euros ; on maximise le revenu net espéré
    max Σ_g [ V_g(x_g) − |x_g| ]   sous   Σ_g |x_g| ≤ B,
V_g étant la valeur exacte de la grille, partage entre nos tickets compris (`net_core`). Trois étages :
1. dans une grille : glouton sur le treillis (copies du même ticket permises), borné par relaxation linéaire (`--bound`) ;
2. entre les grilles : un seuil unique est un prix dual statique (Talluri & van Ryzin 1998) ; le prix dynamique vient de la récurrence
     W_t(b) = (1 − π) W_{t−1}(b) + π · E_g max_{k ≤ b} [ ρ·Ĉ_g(k) − k + W_{t−1}(b − k) ],   W_0 = 0,
   b budget restant, t pas restants du mois, π probabilité d'arrivée d'une grille par pas (Papastavrou, Rajagopalan & Kleywegt 1996 ;
   Lueker 1998 pour le seuil fonction du budget et du temps restants) ;
3. correction de la malédiction de l'optimiseur (Smith & Winkler 2006) : ρ ramène la valeur annoncée Ĉ à la valeur vraie.

Protocole, figé après le prototype sur l'entraînement seul (`docs/sources.md` §69) et avant toute lecture du test :
- grilles : les 409 grilles LF7/LF8 du §47 ; entraînement < 2024 (319), test 2024-2025 (90), lu une seule fois, avec `--test` ;
- entrées de décision : « idéales » (q et S finaux) et « bruitées » (critère), log q̃ = log q + N(0 ; 0,06²) renormalisé (dérive mesurée à
  moins de 6 h de la clôture sur 8 grilles), Ŝ = S · exp N(0 ; 0,25²) (§64), une perturbation par grille, graine fixe ;
- suites de tickets (96 au plus par grille) : « prod » (vivier de 64, tri S3, τ, 16 par grille), « exact » (glouton sur les 3^n tickets,
  sans copie), « copies » (glouton avec copies) ; décision prise en S3 sur les entrées de décision ;
- vérité d'évaluation : tickets évalués sur q et S vrais, partage exact : base, S3 (α = 0,10), S3+ (α = 0,18, moyenne a posteriori du
  mélange p/q sur l'entraînement), public juste (P = q) ;
- règles : seuil statique τ (31 valeurs, 1,0 à 4,0) × plafonds (16, 32, 96) par grille ; prix dynamique × ρ (0,70 à 1,00 par pas de 0,05) ;
- mois : arrivées de `budget_tau.arrivals`, 4 000 années, ordre de clôture ; budget brut sans report ;
- critère : revenu net S3 par an, entrées bruitées, 100 € par mois ; τ*, ρ* et le DP viennent de l'entraînement seul.
Limites : gains espérés du modèle, aucun gain démontré (§37) ; grilles de championnat ; répartition de 2021-2025 ; dérive mesurée sur 8
grilles ; copies supposées admises au guichet (règlement v6, art. 5 et 10.1.5 : ni interdites ni citées).

Usage, depuis scripts/lotofoot, en arrière-plan (progression sur stderr, `tail -f` du journal) :
  ~/.local/share/edge/etudes/venv/bin/python net_revenue.py build 2> ../data/lf/net_revenue.log
  ~/.local/share/edge/etudes/venv/bin/python net_revenue.py train 2>> ../data/lf/net_revenue.log > ../data/lf/net_revenue_train.txt
  ~/.local/share/edge/etudes/venv/bin/python net_revenue.py test 2>> ../data/lf/net_revenue.log > ../data/lf/net_revenue_test.txt
  ~/.local/share/edge/etudes/venv/bin/python net_revenue.py risk 2>> ../data/lf/net_revenue.log > ../data/lf/net_revenue_risk.txt

Amendement du 2026-10-31, après lecture de l'entraînement, avant toute lecture du test : ρ* = 1,00 partout ; règle de production ajoutée à
la comparaison (τ de l'abaque, plafond 16) ; politique principale figée : « copies, sans plafond, prix dynamique, ρ = 1 », et deux repli
(« exact, sans plafond », « copies, plafond 16 ») ; lois annuelles par simulation (commande `risk`, 1 000 tirages, binomiale négative).
Configurations essayées : 4 budgets × 31 τ × 7 règles statiques × 2 modes + 4 budgets × 7 ρ × 4 règles dynamiques, soit 1 848.
"""
import sys, json, os, time, datetime as dt, numpy as np
from multiprocessing import Pool
sys.path.insert(0, ".")
import budget16 as B16, backtest_fdj as BF, budget_tau as BT, net_core as C
from frontier import PRM

K = 96
POOL = 64
SIGMA_Q, SIGMA_S = 0.06, 0.25
ALPHA_PLUS = 0.18
STEPS = 120
TAUS = [round(1.0 + 0.1 * k, 1) for k in range(31)]
CAPS = [16, 32, 96]
RHOS = [0.70, 0.75, 0.80, 0.85, 0.90, 0.95, 1.00]
SEQS = ["prod", "exact", "copies"]
MODES = ["ideal", "noisy"]
TRUTHS = ["base", "s3", "s3p", "null"]
ABAQUE = {60: 1.8, 100: 1.7, 200: 1.5, 400: 1.2}     # règle de production (§49)
NS = 1000
LAG = 2
RATE = (BT.LAM[7] + BT.LAM[8]) / STEPS
BUDGET_MAX = 1.0
CACHE = "../data/lf/net_revenue_cache.npz"
START = time.time()


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - START:6.0f} s] {message}", file=sys.stderr, flush=True)


def inputs(g, mode):
    """Entrées de la décision : q et S vrais, ou perturbés (graine propre à la grille : déterministe)."""
    Q, S = np.array(g["q"]), g["S"]
    if mode == "noisy":
        rng = np.random.default_rng([g["game"], g["gid"], 99])
        Q = Q * np.exp(rng.normal(0, SIGMA_Q, Q.shape)); Q /= Q.sum(axis=1, keepdims=True)
        S = S * float(np.exp(rng.normal(0, SIGMA_S)))
    return Q, S, BF.masses({**g, "S": S})


def pad(values, length, last=None):
    values = list(values)
    return np.array(values + [values[-1] if last is None else last] * (length - len(values)), dtype=np.float64)


def sequences(g, mode):
    """Les trois suites d'une grille et la valeur annoncée de chaque ticket, sur les entrées de décision."""
    P = np.array(g["p"]); prm = PRM[g["game"]]
    Q, S, M = inputs(g, mode)
    table = C.Table(C.s3_probabilities(P, Q), Q, S, M, prm, BT.LOW_MULT)
    pool = [C.code_of(t) for t in B16.greedy(P, Q, S, M, prm, POOL, 1)]
    naive = table.marginals()[pool]
    order = np.argsort(-naive, kind="stable")
    out = {"prod": ([pool[i] for i in order], naive[order].tolist())}
    out["exact"] = table.greedy(K, copies=False)
    out["copies"] = table.greedy(K, copies=True)
    return out


def work(g):
    P, Q0 = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    truths = {"base": C.Table(P, Q0, S, M, prm, 1.0),
              "s3": C.Table(C.s3_probabilities(P, Q0), Q0, S, M, prm, BT.LOW_MULT),
              "s3p": C.Table(C.s3_probabilities(P, Q0, ALPHA_PLUS), Q0, S, M, prm, BT.LOW_MULT),
              "null": C.Table(Q0, Q0, S, M, prm, 1.0)}
    out = {"game": g["game"], "train": g["close"] < "2024", "gid": g["gid"]}
    for mode in MODES:
        seqs = sequences(g, mode)
        for name, (seq, est) in seqs.items():
            out[f"seq_{mode}_{name}"] = pad(seq, K, -1).astype(np.int64)
            out[f"est_{mode}_{name}"] = pad(est, K, 0.0)
            for truth, table in truths.items():
                out[f"cum_{mode}_{name}_{truth}"] = pad(table.walk(seq), K + 1)
    return out


def build():
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    log(f"{len(grids)} grilles, {len(MODES)} modes × {len(SEQS)} suites × {len(TRUTHS)} vérités")
    done = []
    with Pool(8) as pool:
        for k, e in enumerate(pool.imap(work, grids, chunksize=2), 1):
            done.append(e)
            if k % 20 == 0 or k == len(grids):
                log(f"grilles {k} / {len(grids)}, reste environ {(time.time() - START) / k * (len(grids) - k):.0f} s")
    np.savez_compressed(CACHE, **{key: np.array([e[key] for e in done]) for key in done[0]})
    log(f"cache écrit : {CACHE}")


def arrivals(data, mask, seed):
    """Arrivées de `budget_tau.arrivals` et, pour chacune, le nombre de pas restants du mois (ordre de clôture conservé)."""
    G, D = BT.arrivals(data, mask, seed)
    rng = np.random.default_rng(seed + 1000)
    U = np.sort(rng.random(G.shape), axis=1)
    counts = (G >= 0).sum(axis=1)
    for m in range(G.shape[0]):
        U[m, :counts[m]] = np.sort(rng.random(counts[m]))
    return G, D, np.clip(STEPS - (U * STEPS).astype(np.int64), 1, STEPS)


def demand(est, tau, cap):
    """Tickets demandés par grille au seuil τ : la suite s'arrête au premier ticket annoncé sous τ."""
    below = est < tau
    first = np.where(below.any(axis=1), below.argmax(axis=1), est.shape[1])
    return np.minimum(first, cap)


def net_static(data, key, G, tau, budget, cap, truth="s3"):
    est, cum = data[f"est_{key}"], data[f"cum_{key}_{truth}"]
    valid = G >= 0; gi = np.where(valid, G, 0)
    d = np.where(valid, demand(est, tau, cap)[gi], 0)
    before = np.cumsum(d, axis=1) - d
    k = np.clip(budget - before, 0, d)
    net = cum[gi, k] - k
    return float(net.sum() / BT.YEARS), float(k.sum(axis=1).mean()), float((k > 0).sum(axis=1).mean())


def solve_dp(curves, weights, budget, factor=BUDGET_MAX):
    """W[t, b], t = 0…STEPS pas restants, b = 0…bmax euros. `curves` : Ĉ(k) − k des grilles de référence."""
    bmax = int(factor * budget)
    width = curves.shape[1]
    W = np.zeros((STEPS + 1, bmax + 1))
    offsets = np.arange(bmax + 1)[:, None] - np.arange(width)[None, :]    # b − k
    feasible = offsets >= 0
    for t in range(1, STEPS + 1):
        prev = W[t - 1]
        future = np.where(feasible, prev[np.maximum(offsets, 0)], -np.inf)
        best = np.max(curves[:, None, :] + future[None, :, :], axis=2)    # grilles × b
        W[t] = (1 - RATE) * prev + RATE * (weights[:, None] * best).sum(axis=0)
    return W


def net_dynamic(data, key, G, steps, W, rho, budget, cap, truth="s3"):
    """Applique la politique : sur chaque grille, k = argmax ρ·Ĉ(k) − k + W_{t−1}(b − k), Ĉ annoncé sur les entrées de décision."""
    est, cum = data[f"est_{key}"][:, :cap], data[f"cum_{key}_{truth}"]
    head = np.concatenate([np.zeros((len(est), 1)), np.cumsum(est, axis=1)], axis=1)
    curve = rho * head - np.arange(cap + 1)
    valid = G >= 0; gi = np.where(valid, G, 0)
    left = np.full(G.shape[0], budget, dtype=np.int64)
    bmax = W.shape[1] - 1
    net, spent, played = np.zeros(G.shape[0]), np.zeros(G.shape[0]), np.zeros(G.shape[0])
    ks = np.arange(cap + 1)
    for a in range(G.shape[1]):
        live = np.flatnonzero(valid[:, a])
        if not len(live):
            break
        b = left[live][:, None] - ks
        future = W[steps[live, a] - 1][np.arange(len(live))[:, None], np.clip(b, 0, bmax)]    # mois × (k = 0…plafond)
        value = curve[gi[live, a]] + np.where(b >= 0, future, -np.inf)
        k = value.argmax(axis=1)
        net[live] += cum[gi[live, a], k] - k
        spent[live] += k; played[live] += k > 0
        left[live] -= k
    return float(net.sum() / BT.YEARS), float(spent.mean()), float(played.mean())


def prophet(data, key, G, budget, truth="s3", months=4000):
    """Borne d'un clairvoyant qui connaît toutes les grilles du mois : les marginales ≥ 1 les plus hautes, dans la limite du budget."""
    cum = data[f"cum_{key}_{truth}"]
    m = np.diff(cum, axis=1)
    total = 0.0
    for row in G[:months]:
        values = np.sort(m[row[row >= 0]].ravel())[::-1][:budget]
        total += float(np.sum(values[values > 1] - 1))
    return total / months * 12


def weights_of(data, mask):
    w = np.zeros(len(mask))
    for game in (7, 8):
        s = mask & (data["game"] == game)
        w[s] = BT.LAM[game] / (BT.LAM[7] + BT.LAM[8]) / s.sum()
    return w[mask]


def fit_policy(data, key, train, budget, cap, G, steps):
    """ρ* sur l'entraînement : le DP se résout sur ρ·Ĉ, la politique est jugée sur la valeur S3 vraie."""
    rows = []
    for rho in RHOS:
        head = np.concatenate([np.zeros((train.sum(), 1)), np.cumsum(data[f"est_{key}"][train][:, :cap], axis=1)], axis=1)
        curves = rho * head - np.arange(cap + 1)
        W = solve_dp(curves, weights_of(data, train), budget)
        value = net_dynamic(data, key, G, steps, W, rho, budget, cap)
        rows.append((value[0], rho, W, value))
        log(f"  {key} plafond {cap} budget {budget} ρ {rho:.2f} : {value[0]:+.0f} € par an, mises {value[1]:.1f} € par mois")
    return max(rows, key=lambda r: r[0])


def study(split):
    z = np.load(CACHE); data = {k: z[k] for k in z.files}
    data["game"] = data["game"].astype(np.int64)
    train = data["train"].astype(bool); test = ~train
    Gt, Dt, St = arrivals(data, train, 1)
    log("arrivées d'entraînement tirées")
    rows, policies = [], {}
    budgets = (60, 100, 200, 400)
    for budget in budgets:
        for mode in ("noisy", "ideal"):
            for seq in SEQS:
                key = f"{mode}_{seq}"
                for cap in ([16] if seq == "prod" else CAPS):
                    best = max(((net_static(data, key, Gt, t, budget, cap), t) for t in TAUS), key=lambda r: r[0][0])
                    policies[(budget, key, cap, "static")] = best[1]
                    rows.append((budget, key, cap, "static", best[1], best[0]))
                    log(f"{key} plafond {cap} budget {budget} : τ* {best[1]}, {best[0][0]:+.0f} € par an, {best[0][1]:.1f} € par mois")
        for seq in ("exact", "copies"):
            for cap in (16, 96):
                key = f"noisy_{seq}"
                value, rho, W, full = fit_policy(data, key, train, budget, cap, Gt, St)
                policies[(budget, key, cap, "dyn")] = (rho, W)
                rows.append((budget, key, cap, "dyn", rho, full))
    for budget, tau in ABAQUE.items():
        rows.append((budget, "noisy_prod", 16, "abaque", tau, net_static(data, "noisy_prod", Gt, tau, budget, 16)))
    rows.sort(key=lambda r: r[0])
    print("Entraînement, revenu net S3 par an (partage exact)")
    print("budget  suite            plafond règle   réglage   net/an   mises/mois   grilles/mois")
    for r in rows:
        print(f"{r[0]:4d} €  {r[1]:15s} {r[2]:4d}   {r[3]:6s} {r[4]:5.2f}  {r[5][0]:+7.0f} €  {r[5][1]:7.1f} €  {r[5][2]:6.1f}")
    print("\nClairvoyant (borne), entraînement, mode bruité")
    for budget in budgets:
        print(f"{budget:4d} € : " + "  ".join(f"{s} {prophet(data, f'noisy_{s}', Gt, budget):+.0f} €" for s in SEQS))
    if split == "train":
        return
    Gv, Dv, Sv = arrivals(data, test, 2)
    log("arrivées de test tirées : lecture unique du test")
    for budget, tau in ABAQUE.items():
        policies[(budget, "noisy_prod", 16, "abaque")] = tau
    print("\nTest, réglages figés sur l'entraînement : revenu net par an selon la vérité")
    print("budget  suite            plafond règle   réglage " + "  ".join(f"{t:>8s}" for t in TRUTHS) + "   mises/mois  grilles/mois")
    for (budget, key, cap, rule), setting in policies.items():
        if rule in ("static", "abaque"):
            values = [net_static(data, key, Gv, setting, budget, cap, t) for t in TRUTHS]; shown = setting
        else:
            rho, W = setting
            values = [net_dynamic(data, key, Gv, Sv, W, rho, budget, cap, t) for t in TRUTHS]; shown = rho
        print(f"{budget:4d} €  {key:15s} {cap:4d}   {rule:6s} {shown:5.2f}  " + "  ".join(f"{v[0]:+8.0f}" for v in values) + f"   {values[1][1]:7.1f} €  {values[1][2]:6.1f}")
    print("\nClairvoyant (borne), test, mode bruité")
    for budget in budgets:
        print(f"{budget:4d} € : " + "  ".join(f"{s} {prophet(data, f'noisy_{s}', Gv, budget):+.0f} €" for s in SEQS))


def gain_draws(g, T, P, Q, rng, low_mult):
    """Gain brut des k premiers tickets de T (k = 0…K) sur NS tirages (`abaque_vivier.draws`, copies comprises dans c)."""
    n = g["n"]; S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    O = (rng.random((NS, n))[..., None] > np.cumsum(P, axis=1)[None, :, :-1]).sum(axis=2)
    dist = C.poisson_binomial(Q[np.arange(n)[None, :], O])
    good = (T[None, :, :] == O[:, None, :]).sum(axis=2)
    out = np.zeros((NS, T.shape[0] + 1))
    for r in range(len(M)):
        lam = S * dist[:, n - r]
        mu = prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
        mu = np.where(lam < 3, mu * low_mult, mu)
        W = rng.poisson(rng.gamma(BT.TH, mu / BT.TH))[:, None]
        c = np.cumsum(good == n - r, axis=1)
        out[:, 1:] += np.where(c > 0, M[r] * c / (W + c), 0.0)
    return out.astype(np.float32)


def draw_work(item):
    g, seqs = item
    P, Q = np.array(g["p"]), np.array(g["q"])
    rng = np.random.default_rng([g["game"], g["gid"], 7])
    out = {}
    for name, seq in seqs.items():
        seq = seq[seq >= 0]
        T = C.codes(g["n"])[seq]
        sim = gain_draws(g, T, C.s3_probabilities(P, Q), Q, rng, BT.LOW_MULT)
        null = gain_draws(g, T, Q, Q, rng, 1.0)
        pad_to = K + 1 - sim.shape[1]
        out[name] = (np.pad(sim, ((0, 0), (0, pad_to)), mode="edge"), np.pad(null, ((0, 0), (0, pad_to)), mode="edge"))
    return out


def simulate(data, key, sim, G, D, steps, budget, recycled, tau=None, cap=96, W=None):
    """Déroule les mois. Sans `W` : seuil τ ; avec `W` : prix dynamique, ρ = 1. Recyclé : gains rendus au budget LAG grilles plus tard (§67)."""
    est = data[f"est_{key}"][:, :cap]
    head = np.concatenate([np.zeros((len(est), 1)), np.cumsum(est, axis=1)], axis=1)
    curve = head - np.arange(cap + 1)
    valid = G >= 0; gi = np.where(valid, G, 0); di = D % NS
    d = demand(est, tau, cap) if W is None else None
    left = np.full(G.shape[0], budget, dtype=np.int64)
    Ks, gains = np.zeros(G.shape, dtype=np.int64), np.zeros(G.shape)
    ks = np.arange(cap + 1)
    for a in range(G.shape[1]):
        if recycled and a >= LAG:
            left = left + np.floor(gains[:, a - LAG]).astype(np.int64)
        live = np.flatnonzero(valid[:, a])
        if not len(live):
            break
        if W is None:
            k = np.minimum(d[gi[live, a]], np.maximum(left[live], 0))
        else:
            b = left[live][:, None] - ks
            future = W[steps[live, a] - 1][np.arange(len(live))[:, None], np.clip(b, 0, W.shape[1] - 1)]
            k = (curve[gi[live, a]] + np.where(b >= 0, future, -np.inf)).argmax(axis=1)
        Ks[live, a] = k; left[live] -= k
        gains[live, a] = sim[gi[live, a], di[live, a], k]
    return Ks, gains


def describe(label, data, key, G, Ks, gains, null_gains):
    valid = G >= 0; gi = np.where(valid, G, 0)
    net = (gains - Ks).reshape(BT.YEARS, -1).sum(axis=1)
    worst = (null_gains - Ks).reshape(BT.YEARS, -1).sum(axis=1)
    ev = np.where(valid, data[f"cum_{key}_s3"][gi, Ks] - Ks, 0.0).sum() / BT.YEARS
    paid = (gains > 0).reshape(BT.YEARS, -1).sum(axis=1).mean()
    months = (gains > 0).any(axis=1).mean()
    run = np.cumsum((gains - Ks).reshape(BT.YEARS, -1), axis=1)
    drawdown = np.percentile((np.maximum.accumulate(np.maximum(run, 0), axis=1) - run).max(axis=1), 90)
    print(f"{label:34s} {ev:+7.0f} €  {net.mean():+7.0f} €  {net.std():7.0f} €  {np.median(net):+7.0f} €  {np.mean(net < 0):5.1%}  {np.percentile(net, 10):+7.0f} €  {drawdown:6.0f} €"
          f"  {paid:5.2f}  {months:5.1%}  {Ks.sum(axis=1).mean():6.1f} €  {(Ks > 0).sum(axis=1).mean():5.1f}  {Ks[Ks > 0].mean():5.1f}  {worst.mean():+7.0f} €")


def risk():
    """Loi du résultat annuel de la production et du modèle, budget brut et recyclé, entraînement puis test."""
    z = np.load(CACHE); data = {k: z[k] for k in z.files}
    data["game"] = data["game"].astype(np.int64)
    train = data["train"].astype(bool)
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    names = ["noisy_prod", "noisy_exact", "noisy_copies"]
    log(f"tirages : {len(grids)} grilles, {NS} tirages, {len(names)} suites, 2 scénarios")
    items = [(g, {n: data[f"seq_{n}"][i] for n in names}) for i, g in enumerate(grids)]
    with Pool(8) as pool:
        done = pool.map(draw_work, items, chunksize=4)
    sims = {n: (np.stack([d[n][0] for d in done]), np.stack([d[n][1] for d in done])) for n in names}
    log("tirages faits")
    head = "règle                                 EV S3/an  simulé/an  écart type   médiane  P(perte)      p10   repli p90  payées  mois+  mises/mois  grilles/mois  t./grille  public juste"
    for budget in (100, 200):
        tables = {}
        for key, cap in (("noisy_exact", 96), ("noisy_copies", 96), ("noisy_copies", 16)):
            est = data[f"est_{key}"][train][:, :cap]
            curves = np.concatenate([np.zeros((len(est), 1)), np.cumsum(est, axis=1)], axis=1) - np.arange(cap + 1)
            tables[(key, cap, False)] = solve_dp(curves, weights_of(data, train), budget, 1.0)
            tables[(key, cap, True)] = solve_dp(curves, weights_of(data, train), budget, 2.0)
            log(f"programme dynamique résolu : {key} plafond {cap}, {budget} €")
        for label, mask, seed in (("Entraînement", train, 1), ("Test", ~train, 2)):
            G, D, steps = arrivals(data, mask, seed)
            print(f"\n{label}, {budget} € par mois, entrées bruitées, vérité S3 ; public juste : mêmes tickets, issues selon q\n{head}")
            for recycled in (False, True):
                tag = "recyclé" if recycled else "brut"
                for key, null, rule, cap in (("noisy_prod", 0, ABAQUE[budget], 16), ("noisy_exact", 0, None, 96), ("noisy_copies", 0, None, 16), ("noisy_copies", 0, None, 96)):
                    W = None if rule else tables[(key, cap, recycled)]
                    K_, g_ = simulate(data, key, sims[key][0], G, D, steps, budget, recycled, rule, cap, W)
                    _, n_ = simulate(data, key, sims[key][1], G, D, steps, budget, recycled, rule, cap, W)
                    shown = f"{key[6:]} {'τ ' + str(rule) if rule else 'dyn'} plafond {cap} {tag}"
                    describe(shown, data, key, G, K_, g_, n_)
                    log(f"{label} {budget} € {shown}")


if __name__ == "__main__":
    command = sys.argv[1]
    if command == "build":
        build()
    elif command == "risk":
        risk()
    else:
        study(command)
    log("fini")
