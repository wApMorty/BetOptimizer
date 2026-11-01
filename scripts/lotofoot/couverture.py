"""Couvrir les tickets du Loto Foot par des paris simples ou combinés : faisabilité (demande de Paul du 2026-11-01).

Idée de Paul : assurer des gains et réduire la volatilité par des paris de couverture. Un bookmaker ne laisse pas vendre un ticket ; on ne peut
qu'acheter les issues où il perd. Soit U(i) la valeur des tickets si l'issue i d'un nœud de l'arbre se réalise, a(i) = 1 / cote(i), Σ a(i) > 1.
Miser s(i) = a(i) · (H − U(i))⁺ égalise toutes les issues sous le plafond H ; il reste L = H − Σ s(i). dL/dH = 1 − Σ_{U(i) < H} a(i) : on monte H
tant que la somme des a sous H reste inférieure à 1, soit, sur une rencontre, jusqu'à la meilleure issue si son a dépasse la marge :
    L = U_max − Σ_{i ≠ max} a(i) (U_max − U(i)) = Σ_i Q(i) U(i),   Q(max) = a(max) − m,   Q(i) = a(i) ailleurs,   m = Σ a − 1.
En remontant l'arbre des coups d'envoi, L à la racine est le gain assuré (au partage près), sans recours aux probabilités du marché : seules les
cotes et les gains par issue entrent. Dualité : L = min E_Q[G] sur les lois Q telles que Q(pari) ≤ 1 / cote pour tout pari disponible.
Sans intérêt à espérance positive sauf à battre le coût : le ticket vaut V, la couverture complète rend L < V, l'écart est la marge payée.

Protocole, figé avant le calcul :
- grilles : les 409 LF7/LF8 de `net_revenue` ; entraînement < 2024, test 2024-2025, entraînement lu d'abord ; aucun réglage choisi sur le test ;
- tickets : suite « exact » (entrées bruitées), seuil statique τ, plafond 32 ; τ fixé sur l'entraînement pour 94 € de mises par mois (règle
  « exact dyn. 32 brut » du §70) ; budget brut, grille passée si le reste du mois ne suffit pas ; arrivées de `budget_tau.arrivals`, 4 000 années ;
- cotes : a(i) = P(i) · (1 + c(P(i))), c mesurée le 2026-11-01 sur 1 691 relevés de l'archive, meilleur prix de Winamax, Betclic et PMU contre
  Pinnacle sans marge : 14,1 % sous P = 0,2 ; 9,9 % de 0,2 à 0,4 ; 9,1 % de 0,4 à 0,6 ; 7,7 % au-delà. Combiné : produit des a. Variante « marge / 2 »
  purement théorique : aucun bookmaker agréé ne la propose. P est la vérité du scénario (la cote n'y offre aucun biais à exploiter) ;
- décision : valeurs Ĝ tirées des entrées bruitées (log q + N(0 ; 0,06²), S × exp N(0 ; 0,25²)), partage en moyenne ; réalité : q et S vrais, concurrents
  en binomiale négative comme `net_revenue.gain_draws` ;
- calendrier : blocs de coups d'envoi à moins de 110 minutes du premier du bloc, horaires réels ; dans un bloc, couverture par combinés des issues
  du bloc, avant le coup d'envoi. « Sans blocs » (suffixe S) : les rencontres sont traitées l'une après l'autre, aux mêmes marges, ce qui borne par
  excès ce qu'apporterait le pari en direct, dont ni la marge ni la liquidité ne sont mesurées ;
- politiques : aucune ; st1 et st3 (avant le premier coup d'envoi, variance minimale sous contrainte de mises positives, simples seuls, ou simples et
  combinés de 2 ou 3 sélections) ; lastR_X (dernier bloc, si l'écart entre issues dépasse X €) ; lastS_X (dernière rencontre, sans blocs) ; dynR et
  dynS (couverture complète à chaque nœud) ; dynS_25 (nœuds d'écart ≥ 25 €) ; ×0,5 : moitié des mises de dynS ;
- vérités : S3, public juste (P = q), réalité (issues et gagnants publiés, cotes tirées de P) ;
- rapport : espérance exacte, loi annuelle, année en perte, mises de couverture, paris à placer, capital ; gain réel par grille avec bootstrap.
Configurations : 18 politiques, aucune choisie automatiquement. Limites : gains du modèle, marge mesurée sur des ligues de l'archive (les petites
ligues du Loto Foot sont sans doute plus chères), plafonds de mise des opérateurs inconnus, grilles de championnat de 2021-2025.

Amendement du 2026-11-01, après la première lecture de l'entraînement, avant toute lecture du test :
- st1 et st3 : parier les trois signes d'une rencontre n'ajoute aucune variance, le problème est dégénéré et les mises divergeaient ; petite pénalité
  ridge (ε = 10⁻³ fois la diagonale moyenne), sans effet sur la variance atteinte à la précision du tableau ;
- 10 000 tirages par grille au lieu de 1 000, tirés hors de `BT.arrivals` : à 1 000, l'espérance simulée des politiques couvertes s'écartait de 250 € par an
  de l'espérance exacte, tout se jouant sur des rangs rares ;
- ajouts descriptifs : lastS_2000 ×0,5 et ×0,25 (couverture partielle, capital réduit d'autant), détail des grilles réelles où un rang a été touché.

Usage, depuis scripts/lotofoot, en arrière-plan (`tail -f ../data/lf/couverture.log`) :
  ~/.local/share/edge/etudes/venv/bin/python couverture.py check
  ~/.local/share/edge/etudes/venv/bin/python couverture.py train > ../data/lf/couverture_train.txt 2> ../data/lf/couverture.log
  ~/.local/share/edge/etudes/venv/bin/python couverture.py test > ../data/lf/couverture_test.txt 2>> ../data/lf/couverture.log
"""
import sys, json, time, itertools, datetime as dt, numpy as np
from multiprocessing import Pool
from scipy.optimize import nnls
sys.path.insert(0, ".")
import net_revenue as N, budget_tau as BT, net_core as C, backtest_fdj as BF
from frontier import PRM

KEY, CAP, TARGET = "noisy_exact", 32, 94.0
ND = 10000
RIDGE = 1e-3
BLOCK_MINUTES = 110
MIN_BET = 0.10
BINS, SURCHARGE = np.array([0.2, 0.4, 0.6]), np.array([0.1406, 0.0991, 0.0914, 0.0772])
TAUS = [round(1.5 + 0.05 * k, 2) for k in range(31)]
START = time.time()


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - START:6.0f} s] {message}", file=sys.stderr, flush=True)


def book(P, scale=1.0):
    """a(i) = 1 / cote : probabilité de la vérité, plus la surcharge mesurée par tranche."""
    return P * (1 + scale * SURCHARGE[np.digitize(P, BINS)])


def blocks_of(g):
    """Rencontres groupées par bloc de coups d'envoi, dans l'ordre du calendrier."""
    t = [dt.datetime.fromisoformat(s.replace("Z", "+00:00")) for s in g["start"]]
    out = []
    for i in sorted(range(g["n"]), key=lambda i: (t[i], i)):
        if out and (t[i] - t[out[-1][0]]).total_seconds() < BLOCK_MINUTES * 60:
            out[-1].append(i)
        else:
            out.append([i])
    return out


def kron_rows(rows):
    out = np.ones(1)
    for row in rows:
        out = np.kron(out, row)
    return out


def tree(G, a, p, blocks, start, spread):
    """Couverture sur l'arbre : renvoie, par issue finale (code base 3), le résultat des paris, les mises, le nombre de paris et le capital, puis L racine."""
    n = a.shape[0]
    order = [i for b in blocks for i in b]
    perm = [n - 1 - i for i in order]                      # l'axe j du tenseur porte la rencontre n − 1 − j
    U = G.reshape((3,) * n).transpose(perm).reshape(-1)
    plan = []
    for b in range(len(blocks) - 1, -1, -1):
        size = 3 ** len(blocks[b])
        U = U.reshape(-1, size)
        ac, pc = kron_rows(a[blocks[b]]), kron_rows(p[blocks[b]])
        value = U @ pc
        stakes = None
        if b >= start:
            rank = np.argsort(U, axis=1)
            Us = np.take_along_axis(U, rank, axis=1)
            hurdle_at = np.minimum((np.cumsum(ac[rank], axis=1) >= 1).argmax(axis=1), size - 1)
            H = Us[np.arange(len(U)), hurdle_at]
            stakes = ac * np.maximum(H[:, None] - U, 0.0)
            locked = H - stakes.sum(axis=1)
            live = (U.max(axis=1) - U.min(axis=1)) >= spread
            stakes[~live] = 0.0
            value = np.where(live, locked, value)
        plan.append((stakes, ac, size))
        U = value
    root = float(U[0])
    result, turnover, bets, cash, need = (np.zeros(1) for _ in range(5))
    for stakes, ac, size in reversed(plan):
        if stakes is None:
            stakes = np.zeros((len(result), size))
        total = stakes.sum(axis=1)
        need = np.maximum(need, total - cash)
        back = stakes / ac
        result = (result - total)[:, None] + back
        cash = (cash - total)[:, None] + back
        turnover = np.repeat(turnover + total, size).reshape(-1, size)
        bets = np.repeat(bets + (stakes >= MIN_BET).sum(axis=1), size).reshape(-1, size)
        need = np.repeat(need, size).reshape(-1, size)
        result, cash, turnover, bets, need = (x.reshape(-1) for x in (result, cash, turnover, bets, need))
    back_perm = np.argsort(perm)
    restore = lambda x: x.reshape((3,) * n).transpose(back_perm).reshape(-1)
    return restore(result), restore(turnover), restore(bets), restore(need), root


def static_features(a, legs):
    """Gains nets, par issue, des simples (legs = 1) et combinés de 2 ou 3 sélections disponibles."""
    n = a.shape[0]
    O = C.codes(n)
    cols = []
    for size in range(1, legs + 1):
        for matches in itertools.combinations(range(n), size):
            for signs in itertools.product(range(3), repeat=size):
                hit = np.all(O[:, matches] == np.array(signs), axis=1)
                cols.append(hit / np.prod(a[list(matches), list(signs)]) - 1.0)
    return np.stack(cols, axis=1)


def static_hedge(G, a, p, legs):
    """Mises positives de variance minimale avant le premier coup d'envoi."""
    O = C.codes(a.shape[0])
    pi = np.prod(p[np.arange(a.shape[0])[None, :], O], axis=1)
    B = static_features(a, legs)
    root = np.sqrt(pi)[:, None]
    A = (B - pi @ B) * root
    y = (G - pi @ G) * root[:, 0]
    eps = RIDGE * float((A * A).sum(axis=0).mean())
    Qm, R = np.linalg.qr(np.vstack([A, np.sqrt(eps) * np.eye(A.shape[1])]), mode="reduced")
    stakes, _ = nnls(R, -(Qm.T @ np.concatenate([y, np.zeros(A.shape[1])])), maxiter=20 * R.shape[1])
    return B @ stakes, stakes.sum(), int((stakes >= MIN_BET).sum())


def leaf_values(table, seq):
    table.reset()
    for t in seq:
        table.add(int(t))
    idx = np.arange(len(table.prob))
    values = np.zeros(len(table.prob))
    for r, c in enumerate((table.c0, table.c1)):
        F = np.concatenate([np.zeros((len(idx), 1)), np.cumsum(table.dF[r], axis=1)], axis=1)
        values += table.M[r] * F[idx, c]
    c0, c1 = table.c0.copy(), table.c1.copy()
    table.reset()
    return values, c0, c1


def policies(Ghat, a_book, a_half, phat, real_blocks, n, with_static):
    """Résultat des paris par issue pour chaque politique."""
    seq_blocks = [[i] for b in real_blocks for i in b]
    R, S = len(real_blocks), len(seq_blocks)
    out = {}
    for X in (100, 500, 2000):
        out[f"lastR_{X}"] = tree(Ghat, a_book, phat, real_blocks, R - 1, X)
        out[f"lastS_{X}"] = tree(Ghat, a_book, phat, seq_blocks, S - 1, X)
    out["dynR"] = tree(Ghat, a_book, phat, real_blocks, 0, 0.0)
    out["dynS"] = tree(Ghat, a_book, phat, seq_blocks, 0, 0.0)
    out["dynS_25"] = tree(Ghat, a_book, phat, seq_blocks, 0, 25.0)
    out["dynR_m2"] = tree(Ghat, a_half, phat, real_blocks, 0, 0.0)
    out["dynS_m2"] = tree(Ghat, a_half, phat, seq_blocks, 0, 0.0)
    out["lastS_500_m2"] = tree(Ghat, a_half, phat, seq_blocks, S - 1, 500)
    if with_static:
        for legs, name in ((1, "st1"), (3, "st3")):
            h, total, count = static_hedge(Ghat, a_book, phat, legs)
            out[name] = (h, np.full(h.shape, total), np.full(h.shape, float(count)), np.full(h.shape, total), 0.0)
    return out


def work(item):
    g, seq = item
    k = len(seq)
    if k == 0:
        return None
    n, S = g["n"], g["S"]
    P, Q0 = np.array(g["p"]), np.array(g["q"])
    M = BF.masses(g); prm = PRM[g["game"]]
    Qn, Sn, Mn = N.inputs(g, "noisy")
    phat = C.s3_probabilities(P, Qn)
    Ghat, _, _ = leaf_values(C.Table(phat, Qn, Sn, Mn, prm, BT.LOW_MULT), seq)
    rng = np.random.default_rng([g["game"], g["gid"], 2026])
    blocks = blocks_of(g)
    out = {"k": k, "n": n, "blocks": len(blocks)}
    truths = {"s3": (C.s3_probabilities(P, Q0), BT.LOW_MULT), "null": (Q0, 1.0)}
    for truth, (Pt, low) in truths.items():
        table = C.Table(Pt, Q0, S, M, prm, low)
        Gt, c0, c1 = leaf_values(table, seq)
        pol = policies(Ghat, book(Pt), book(Pt, 0.5), phat, blocks, n, truth == "s3")
        pi = table.prob
        O = (rng.random((ND, n))[..., None] > np.cumsum(Pt, axis=1)[None, :, :-1]).sum(axis=2)
        idx = O @ 3 ** np.arange(n)
        gain = np.zeros(ND)
        for r, c in enumerate((c0, c1)):
            W = rng.poisson(rng.gamma(BT.TH, table.mu[r][idx] / BT.TH))
            gain += np.where(c[idx] > 0, M[r] * c[idx] / (W + c[idx]), 0.0)
        res = {"gain": gain.astype(np.float32), "ev": float(pi @ Gt) - k, "paid": (c0[idx] + c1[idx] > 0)}
        for name, (h, turn, bets, need, root) in pol.items():
            res[name] = dict(h=h[idx].astype(np.float32), ev=float(pi @ h), turn=float(pi @ turn), bets=float(pi @ bets),
                             need=need[idx].astype(np.float32), need_max=float(need[pi > 0].max()), root=root)
        out[truth] = res
    real_table = C.Table(P, Q0, S, M, prm, 1.0)
    pol = policies(Ghat, book(P), book(P, 0.5), phat, blocks, n, True)
    o = C.code_of(g["o"])
    _, c0, c1 = leaf_values(real_table, seq)
    W = {n - r["good"]: r["W"] for r in g["ranks"]}
    gain = 0.0
    if c0[o] and 0 in W: gain += M[0] * c0[o] / (W[0] + c0[o])
    if c1[o] and 1 in W: gain += M[1] * c1[o] / (W[1] + c1[o])
    out["real"] = {"gain": float(gain), **{name: float(v[0][o]) for name, v in pol.items()}}
    return out


def load():
    z = np.load(N.CACHE)
    data = {key: z[key] for key in z.files if key in ("game", "train", "gid", f"seq_{KEY}", f"est_{KEY}")}
    data["game"] = data["game"].astype(np.int64)
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    return data, grids


def month_spend(data, mask, tau, seed):
    """Tickets joués par grille et mises mensuelles au seuil τ, grille passée si le reste du mois ne suffit pas."""
    G, D = BT.arrivals(data, mask, seed)
    est = data[f"est_{KEY}"][:, :CAP]
    below = est < tau
    need = np.where(below.any(axis=1), below.argmax(axis=1), CAP)
    valid = G >= 0; gi = np.where(valid, G, 0)
    left = np.full(G.shape[0], 100, dtype=np.int64)
    K = np.zeros(G.shape, dtype=np.int64)
    for s in range(G.shape[1]):
        want = np.where(valid[:, s], need[gi[:, s]], 0)
        play = want <= left
        K[:, s] = np.where(play, want, 0)
        left -= K[:, s]
    return G, D, K, need


def choose_tau(data):
    train = data["train"].astype(bool)
    best = None
    for tau in TAUS:
        _, _, K, _ = month_spend(data, train, tau, 1)
        spend = K.sum(axis=1).mean()
        log(f"τ = {tau} : {spend:.1f} € de mises par mois")
        if best is None or abs(spend - TARGET) < abs(best[1] - TARGET):
            best = (tau, spend)
    return best[0]


def year_table(data, results, mask, tau, seed, truth, names):
    G, D, K, _ = month_spend(data, mask, tau, seed)
    played = K > 0; gi = np.where(played, G, 0)
    di = np.random.default_rng(seed + 500).integers(0, ND, G.shape)      # `BT.arrivals` ne tire que dans 0…1 999
    empty = np.zeros(ND, dtype=np.float32)
    rows = lambda field: np.stack([r[truth][field] if r else empty for r in results])
    gain = rows("gain")
    base = np.where(played, gain[gi, di] - K, 0.0)
    year = lambda x: x.sum(axis=1).reshape(BT.YEARS, 12, -1).sum(axis=(1, 2)) if x.ndim == 2 else x
    ev0 = np.array([r[truth]["ev"] if r else 0.0 for r in results])
    head = f"{'politique':16s} {'espérance/an':>12s} {'simulé/an':>10s} {'écart type':>10s} {'médiane':>9s} {'P(perte)':>9s} {'p5':>8s} {'p95':>9s} {'coût/an':>9s} {'mises couv./an':>14s} {'paris/an':>9s} {'capital p99':>11s} {'capital max':>11s}"
    print(head)
    y0 = year(base)
    ev_none = float(np.where(played, ev0[gi], 0.0).sum() / BT.YEARS)
    print(f"{'aucune':16s} {ev_none:+11.0f} € {y0.mean():+9.0f} € {y0.std():9.0f} € {np.median(y0):+8.0f} € {np.mean(y0 < 0) * 100:8.1f}% {np.percentile(y0, 5):+7.0f} € {np.percentile(y0, 95):+8.0f} €")
    for name, f in names:
        H = np.stack([r[truth][name]["h"] if (r and name in r[truth]) else empty for r in results])
        field = lambda key: np.array([r[truth][name][key] if (r and name in r[truth]) else 0.0 for r in results])
        y = year(np.where(played, base + f * H[gi, di], 0.0))
        ev = float(np.where(played, ev0[gi] + f * field("ev")[gi], 0.0).sum() / BT.YEARS)
        cost = ev_none - ev
        turn = f * float(np.where(played, field("turn")[gi], 0.0).sum() / BT.YEARS)
        bets = float(np.where(played, field("bets")[gi], 0.0).sum() / BT.YEARS)
        need = np.stack([r[truth][name]["need"] if (r and name in r[truth]) else empty for r in results])
        cap_year = (np.where(played, need[gi, di], 0.0)).reshape(BT.YEARS, -1).max(axis=1) * f
        label = name if f == 1 else f"{name} ×{f}"
        print(f"{label:16s} {ev:+11.0f} € {y.mean():+9.0f} € {y.std():9.0f} € {np.median(y):+8.0f} € {np.mean(y < 0) * 100:8.1f}% {np.percentile(y, 5):+7.0f} € {np.percentile(y, 95):+8.0f} € {cost:8.0f} € {turn:13.0f} € {bets:9.0f} {np.percentile(cap_year, 99):10.0f} € {cap_year.max():10.0f} €")


def lock_table(results, mask):
    """Valeur assurée par la couverture complète, grille par grille, rapportée aux mises."""
    print("\nGain assuré en moyenne de partage, par euro de tickets (L racine / mises), S3 :")
    print(f"{'politique':12s} {'médiane':>8s} {'moyenne':>8s} {'p90':>7s} {'max':>7s} {'part > 1':>9s} {'valeur V / mises':>17s}")
    chosen = [r for r, m in zip(results, mask) if m and r]
    V = np.array([(r["s3"]["ev"] + r["k"]) / r["k"] for r in chosen])
    for name in ("dynR", "dynS", "dynR_m2", "dynS_m2"):
        x = np.array([r["s3"][name]["root"] / r["k"] for r in chosen])
        print(f"{name:12s} {np.median(x):8.3f} {x.mean():8.3f} {np.percentile(x, 90):7.3f} {x.max():7.3f} {np.mean(x > 1) * 100:8.1f}% {V.mean():17.3f}")
    print(f"grilles jouées : {len(chosen)} ; blocs par grille : {np.mean([r['blocks'] for r in chosen]):.2f}")


def real_table(results, mask, names):
    """Résultat réel, grille par grille, sans budget : tickets, puis paris de couverture décidés sur les entrées bruitées."""
    chosen = [r for r, m in zip(results, mask) if m and r]
    k = np.array([r["k"] for r in chosen], float); gain = np.array([r["real"]["gain"] for r in chosen])
    print(f"\nRéalité sans budget : {len(chosen)} grilles, {k.sum():.0f} € de tickets, {gain.sum():.0f} € de gains, {int((gain > 0).sum())} grilles payées")
    print(f"{'politique':16s} {'net':>10s} {'net / €':>8s} {'IC 95 % (bootstrap de grilles)':>32s} {'écart type/grille':>18s}")
    print("Grilles où un rang a été touché ou couverture de dernière rencontre engagée (lastS_2000, dynS) :")
    for r in chosen:
        if r["real"]["gain"] > 0 or abs(r["real"].get("lastS_2000", 0.0)) > 1:
            print(f"  LF{r['n']}, {r['k']} tickets : gain {r['real']['gain']:.0f} €, dernière rencontre {r['real'].get('lastS_2000', 0.0):+.0f} €, complète {r['real'].get('dynS', 0.0):+.0f} €")
    rng = np.random.default_rng(13)
    picks = rng.integers(0, len(k), (5000, len(k)))
    for name, f in [("aucune", 0.0)] + names:
        h = np.array([r["real"].get(name, 0.0) for r in chosen]) * f
        net = gain - k + h
        boot = net[picks].sum(axis=1) / k[picks].sum(axis=1)
        label = name if f in (0.0, 1.0) else f"{name} ×{f}"
        print(f"{label:16s} {net.sum():+9.0f} € {net.sum() / k.sum():+8.3f} [{np.percentile(boot, 2.5):+7.3f} ; {np.percentile(boot, 97.5):+7.3f}] {net.std():17.1f} €")


def run(split):
    data, grids = load()
    tau = choose_tau(data)
    log(f"τ retenu : {tau}")
    train = data["train"].astype(bool)
    mask = train if split == "train" else ~train
    below = data[f"est_{KEY}"][:, :CAP] < tau
    need = np.where(below.any(axis=1), below.argmax(axis=1), CAP)
    items = [(g, data[f"seq_{KEY}"][i][:need[i]]) for i, g in enumerate(grids)]
    results, done = [None] * len(items), 0
    todo = [i for i in range(len(items)) if mask[i]]
    log(f"{len(todo)} grilles à traiter, dont {sum(need[i] > 0 for i in todo)} jouées au seuil τ")
    with Pool(8) as pool:
        for i, r in zip(todo, pool.imap(work, [items[i] for i in todo], chunksize=2)):
            results[i] = r; done += 1
            if done % 10 == 0 or done == len(todo):
                log(f"grilles {done} / {len(todo)}, reste environ {(time.time() - START) / done * (len(todo) - done):.0f} s")
    print(f"{split}, τ = {tau}, plafond {CAP}, 100 € par mois, budget brut")
    names = [(n, 1.0) for n in ("st1", "st3", "lastR_100", "lastR_500", "lastR_2000", "lastS_100", "lastS_500", "lastS_2000", "dynR", "dynS", "dynS_25")]
    names += [("lastS_2000", 0.5), ("lastS_2000", 0.25), ("dynS", 0.5), ("dynR_m2", 1.0), ("dynS_m2", 1.0), ("lastS_500_m2", 1.0)]
    seed = 1 if split == "train" else 2
    print("\nVérité S3")
    year_table(data, results, mask, tau, seed, "s3", names)
    log("S3 fait")
    print("\nPublic juste (P = q, cotes tirées de q)")
    year_table(data, results, mask, tau, seed, "null", [x for x in names if x[0] not in ("st1", "st3")])
    log("public juste fait")
    lock_table(results, mask)
    real_table(results, mask, names)


def check():
    """Contrôles : la couverture complète égalise les issues et l'espérance retrouve celle du cache."""
    data, grids = load()
    i = next(j for j, g in enumerate(grids) if g["game"] == 7 and data["train"][j])
    g = grids[i]
    P, Q = np.array(g["p"]), np.array(g["q"]); n = g["n"]
    rng = np.random.default_rng(3)
    G = np.maximum(rng.normal(0, 40, 3 ** n), 0.0)
    a = book(P)
    for blocks in ([[j] for j in range(n)], blocks_of(g), [list(range(n))]):
        h, _, _, _, root = tree(G, a, P, blocks, 0, 0.0)
        total = G + h
        assert total.min() >= root - 1e-6, (total.min(), root)
        print("blocs", [len(b) for b in blocks], "L =", round(root, 3), "min", round(total.min(), 3), "max", round(total.max(), 3))
    # Une rencontre : formule fermée L = Σ Q(i) U(i).
    a1 = book(P[:1]); U = np.array([100.0, 10.0, 30.0]).reshape(1, 3)
    h, _, _, _, root = tree(np.tile(U[0], 1), a1, P[:1], [[0]], 0, 0.0)
    Qm = a1[0].copy(); Qm[0] -= a1[0].sum() - 1
    assert abs(root - float(Qm @ U[0])) < 1e-9, (root, Qm @ U[0])
    print("formule fermée vérifiée :", round(root, 3))
    # Variance minimale : ne doit jamais augmenter la variance.
    H, total, count = static_hedge(G, a, P, 1)
    pi = np.prod(P[np.arange(n)[None, :], C.codes(n)], axis=1)
    var = lambda x: float(pi @ x ** 2 - (pi @ x) ** 2)
    assert var(G + H) <= var(G) + 1e-6
    print("variance", round(var(G), 1), "→", round(var(G + H), 1), "mises", round(total, 2), "paris", count)
    H, total, count = static_hedge(G, a, P, 3)
    assert var(G + H) <= var(G) + 1e-6 and total < 1e4, (var(G + H), total)
    print("variance", round(var(G), 1), "→", round(var(G + H), 1), "mises", round(total, 2), "paris", count)
    # Mêmes tickets : l'espérance S3 doit retrouver le cumul du cache.
    z = np.load(N.CACHE)
    seq = z[f"seq_{KEY}"][i][:8]
    table = C.Table(C.s3_probabilities(P, Q), Q, g["S"], BF.masses(g), PRM[7], BT.LOW_MULT)
    values, _, _ = leaf_values(table, seq)
    cached = float(z[f"cum_{KEY}_s3"][i][8])
    assert abs(float(table.prob @ values) - cached) < 1e-6 * max(cached, 1), (table.prob @ values, cached)
    print("espérance retrouvée :", round(cached, 3))


if __name__ == "__main__":
    if sys.argv[1] == "check":
        check()
    else:
        run(sys.argv[1])
    log("fini")
