"""Au-delà de 16 tickets par grille : que rapporterait de plus chaque ticket, partage entre nos propres tickets déduit ? (2026-10-30)

Protocole, figé avant le calcul :
- grilles : les 409 grilles LF7/LF8 de l'étude FDJ (`grids_fdj.json`, vraie répartition finale), plus les grilles en vigueur du
  journal `lotofoot_reco` (répartition de l'heure du calcul), pour contrôle et lecture, sans servir au choix ;
- tickets : la recherche de production (gloutonne, μ = 10, tickets distincts) prolongée à 64 tickets. Les 16 premiers restent ceux
  de la production, classés par espérance S3 décroissante ; les suivants sont classés entre eux de la même façon, après eux ;
- valeur d'un ensemble T, exacte, sans simulation, sur les 3^n issues o :
    V(T) = Σ_o P(o) Σ_r M_r · c_r(o) · E[1 / (c_r(o) + W)],  W ~ Poisson(μ_r(o)),
  avec c_r(o) le nombre de nos tickets au rang r pour l'issue o et μ_r(o) = κ λ^β + φ S / 3^n (× 1,4 si λ < 3 en S3). Pour c = 1 on
  retrouve l'espérance de production `(1 − e^{−μ}) / μ` ; deux rangs seulement (sans-faute, une erreur), et deux tickets distincts
  ne sont jamais parfaits ensemble : le partage entre nous ne porte que sur le rang à une erreur ;
- valeur marginale du k-ième ticket : V(T_k) − V(T_{k−1}), à comparer à son espérance « naïve » (celle de la production) ;
- règles : « naïve » (espérance S3 du ticket ≥ τ, celle de la production) et « marginale » (valeur marginale S3 ≥ τ, ticket sauté
  sinon) ;
- mois simulés : ceux de `budget_tau.py` (mêmes rythmes, ordre aléatoire, budget mensuel sans report, 4 000 années, arrivées communes) ;
- configurations : 2 règles × 5 plafonds par grille (16, 24, 32, 48, 64) × 4 budgets (60, 100, 200, 400 €) × 31 τ, soit 1 240 ;
- critère : τ* maximise V S3 nette par an (partage déduit) sur l'entraînement (grilles closes avant 2024) ; test lu ensuite ;
- contrôle réel, sans servir au choix : gains qu'auraient versés les tickets, depuis les issues et les gagnants publiés.
Ajouts après lecture de la première passe (`beyond_16_passe1.txt`, entraînement et test déjà vus, à compter comme tels) :
- règle « vivier » : mêmes 64 candidats, classés par espérance S3 sans distinguer les 16 premiers (620 configurations de plus, soit
  1 860 essayées en tout ; la simulation de la règle marginale, identique à la naïve à 1 % près, n'est pas refaite) ;
- calibration : rangs attendus et atteints, gains réels par euro et intervalle à 95 % (bootstrap par grilles), tickets du seuil de
  production, 16 premiers candidats contre suivants.
Limites : espérances du modèle, aucun gain démontré (le test figé du §37 a échoué) ; répartition finale pour l'historique ; grilles de
championnat seulement ; mois réels non indépendants ; Poisson pour W (la binomiale négative des simulations est plus dispersée).

Usage, depuis scripts/lotofoot, en arrière-plan :
  ~/.local/share/edge/etudes/venv/bin/python beyond_16.py 2> ../data/lf/beyond_16.log &
Progression sur stderr ; tables sur stdout ; données dans ../data/lf/beyond_16.json.
"""
import sys, json, sqlite3, time, datetime as dt, os, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast, budget16 as B16
import backtest_fdj as BF
from frontier import PRM

KMAX, HEAD, YEARS = 64, 16, 4000
SHRINK, LOW_MULT = 0.1, 1.4
CAPS = [16, 24, 32, 48, 64]
BUDGETS = [60, 100, 200, 400]
TAUS = [round(1.0 + 0.1 * k, 1) for k in range(31)]
TAU_PROD = 1.7
RULES = ["naïve", "marginale", "vivier"]
SWEPT = [0, 2]
BOOTSTRAP = 5000
LAM = {7: 3.2 * 30.44 / 7, 8: 3.4 * 30.44 / 7}
NODES, WEIGHTS = np.polynomial.legendre.leggauss(64)
NODES, WEIGHTS = (NODES + 1) / 2, WEIGHTS / 2      # de [−1, 1] à [0, 1]
OUT = "../data/lf/beyond_16.json"
START = time.time()


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - START:6.0f} s] {message}", file=sys.stderr, flush=True)


def share_table(mu):
    """F[…, c] = c · E[1 / (c + W)], W ~ Poisson(μ), c = 0…KMAX.

    E[1 / (c + W)] = ∫₀¹ x^{c−1} e^{−μ(1−x)} dx, Gauss-Legendre sur [a, 1] avec a = max(0, 1 − 40/μ) : l'exponentielle est piquée
    en x = 1 et ce qu'on laisse de côté pèse moins de e^{−40}. Pour c = 1 : (1 − e^{−μ}) / μ, l'espérance de production."""
    start = np.maximum(0.0, 1 - 40 / np.maximum(mu, 1e-300))[..., None]
    x = start + (1 - start) * NODES                                    # … × nœuds
    weight = (1 - start) * WEIGHTS * np.exp(-mu[..., None] * (1 - x))
    out = np.zeros(mu.shape + (KMAX + 1,))
    for c in range(1, KMAX + 1):                                       # `weight` porte x^{c−1} : un facteur x à chaque tour
        out[..., c] = c * weight.sum(axis=-1)
        weight = weight * x
    return out


class Exact:
    """Tables exactes d'une grille dans un scénario : probabilité de chaque issue, F par issue et par rang."""

    def __init__(self, P, Q, S, M, prm, low_mult):
        n = P.shape[0]
        self.n, self.M = n, np.asarray(M)
        self.powers = 3 ** np.arange(n)
        codes = np.arange(3 ** n)
        O = (codes[:, None] // self.powers) % 3
        idx = np.arange(n)[None, :]
        self.prob = np.prod(P[idx, O], axis=1)
        a = Q[idx, O]
        dist = np.zeros((len(O), n + 1)); dist[:, 0] = 1.0
        for i in range(n):
            x = a[:, i:i + 1]
            dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
        self.F = []
        for r in range(len(M)):
            lam = S * dist[:, n - r]
            mu = prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
            self.F.append(share_table(np.where(lam < 3, mu * low_mult, mu)))
        self.reset()

    def reset(self):
        self.count = [np.zeros(3 ** self.n, dtype=np.int64) for _ in self.M]

    def neighbours(self, t):
        """Issues où le ticket fait 0 erreur, puis exactement 1 erreur (deux rangs payés au LF7 et au LF8)."""
        t = np.asarray(t)
        code = int(t @ self.powers)
        ones = np.array([code + ((t[i] + d) % 3 - t[i]) * self.powers[i] for i in range(self.n) for d in (1, 2)])
        return [np.array([code]), ones]

    def naive(self, t):
        return sum(float(np.sum(self.prob[o] * self.M[r] * self.F[r][o, 1])) for r, o in enumerate(self.neighbours(t)))

    def marginal(self, t):
        total = 0.0
        for r, o in enumerate(self.neighbours(t)):
            c = self.count[r][o]
            total += float(np.sum(self.prob[o] * self.M[r] * (self.F[r][o, c + 1] - self.F[r][o, c])))
        return total

    def add(self, t):
        for r, o in enumerate(self.neighbours(t)):
            self.count[r][o] += 1

    def covered(self):
        return float(np.sum(self.prob[(self.count[0] + self.count[1]) > 0]))


def tickets(P, Q, S, M, prm):
    """Recherche de production prolongée ; S3 décroissante à l'intérieur des 16 premiers, puis à l'intérieur des suivants."""
    T = B16.greedy(P, Q, S, M, prm, KMAX, 1)
    Pt = (1 - SHRINK) * P + SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    return T, Pt


def order_by_s3(T, s3):
    head = sorted(range(HEAD), key=lambda k: -s3[k])
    tail = sorted(range(HEAD, len(T)), key=lambda k: -s3[k])
    return head + tail


def walk(base, worse, T, s3, tau, rule):
    """Parcourt la liste ordonnée au seuil τ. Renvoie les valeurs cumulées (V S3, V base, naïve S3) des 0, 1, 2… tickets joués."""
    base.reset(); worse.reset()
    v_s3, v_base, naive = [0.0], [0.0], [0.0]
    covered = [0.0]
    for t, value in zip(T, s3):
        if value < tau:
            continue
        gain = worse.marginal(t)
        if rule == 1 and gain < tau:
            continue
        v_s3.append(v_s3[-1] + gain)
        v_base.append(v_base[-1] + base.marginal(t))
        naive.append(naive[-1] + value)
        worse.add(t); base.add(t)
        covered.append(base.covered())
    return v_s3, v_base, naive, covered


def pad(values):
    return np.array(values + [values[-1]] * (KMAX + 1 - len(values)), dtype=np.float64)


def realised(g, T, M):
    """Gains qu'auraient versés les tickets de T, dans l'ordre, sur la grille `g` : issue et gagnants publiés (0 sans publication)."""
    n, out = g["n"], []
    winners = {n - r["good"]: r["W"] for r in g["ranks"]}
    errors = [n - sum(a == b for a, b in zip(t, g["o"])) for t in T]
    for k in range(1, len(T) + 1):
        total = 0.0
        for r, mass in enumerate(M):
            c = sum(e == r for e in errors[:k])
            if c and r in winners:
                total += mass * c / (winners[r] + c)
        out.append(total)
    return out


def calibration(g, T, s3, base, M):
    """Un enregistrement par ticket au seuil de production : tête (16 premiers candidats) ou queue, rangs attendus et atteints, gain seul."""
    n, rows = g["n"], []
    winners = {n - r["good"]: r["W"] for r in g["ranks"]}
    for position, (t, value) in enumerate(zip(T, s3)):
        if value < TAU_PROD:
            continue
        near = base.neighbours(t)
        errors = n - sum(a == b for a, b in zip(t, g["o"]))
        gain = M[errors] / (winners[errors] + 1) if errors < len(M) and errors in winners else 0.0
        rows.append((position < HEAD, float(base.prob[near[0]].sum()), float(base.prob[near[1]].sum()), base.naive(t), errors, gain))
    return rows


def ratio_ci(gains, stakes):
    """Gains par euro et intervalle à 95 % : on rééchantillonne les grilles, seule unité indépendante."""
    rng = np.random.default_rng(7)
    picks = rng.integers(0, len(gains), (BOOTSTRAP, len(gains)))
    ratios = gains[picks].sum(axis=1) / np.maximum(stakes[picks].sum(axis=1), 1)
    return gains.sum() / max(stakes.sum(), 1), np.percentile(ratios, 2.5), np.percentile(ratios, 97.5)


def calibration_table(ev, label):
    print(f"\n{label} : tickets ≥ {TAU_PROD}, attendu contre réel (modèle de base, sans partage entre nous)")
    print("groupe   tickets   sans-faute att./réel   une erreur att./réel   EV base annoncée   gains réels / €   IC 95 %")
    for head, name in ((True, "tête"), (False, "queue")):
        rows = [[r for r in e["calibration"] if r[0] == head] for e in ev]
        flat = [r for line in rows for r in line]
        if not flat:
            continue
        gains = np.array([sum(r[5] for r in line) for line in rows]); stakes = np.array([len(line) for line in rows], dtype=float)
        ratio, low, high = ratio_ci(gains, stakes)
        print(f"{name:7s} {len(flat):6d}     {sum(r[1] for r in flat):6.2f} / {sum(r[4] == 0 for r in flat):3d}"
              f"         {sum(r[2] for r in flat):7.1f} / {sum(r[4] == 1 for r in flat):4d}        {np.mean([r[3] for r in flat]):7.2f}"
              f"           {ratio:6.2f}         [{low:.2f} ; {high:.2f}]")


def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    T, Pt = tickets(P, Q, S, M, prm)
    base, worse = Exact(P, Q, S, M, prm, 1.0), Exact(Pt, Q, S, M, prm, LOW_MULT)
    s3_raw = [worse.naive(t) for t in T]
    order = order_by_s3(T, s3_raw)
    T = [T[k] for k in order]; s3 = [s3_raw[k] for k in order]
    naive_base = [base.naive(t) for t in T]
    # Liste complète, sans seuil : valeur marginale de chaque position.
    v_s3, v_base, _, _ = walk(base, worse, T, s3, -1.0, 0)
    value = {"game": g["game"], "train": g["close"] < "2024", "s3": s3, "naive_base": naive_base,
             "marginal_s3": np.diff(v_s3).tolist(), "marginal_base": np.diff(v_base).tolist(),
             "ctrl_ev": lf_fast.ev_p(T[0], Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=LOW_MULT)[0] - s3[0]}
    tables = np.zeros((3, len(TAUS), KMAX + 1, 2))
    demand = np.zeros((3, len(TAUS)), dtype=np.int64)
    pool = sorted(range(len(T)), key=lambda k: -s3[k])
    T_pool, s3_pool = [T[k] for k in pool], [s3[k] for k in pool]
    lists = [(T, s3, 0), (T, s3, 1), (T_pool, s3_pool, 0)]
    for rule, (tickets_, values, skip) in enumerate(lists):
        for i, tau in enumerate(TAUS):
            a, b, _, _ = walk(base, worse, tickets_, values, tau, skip)
            tables[rule, i, :, 0] = pad(a); tables[rule, i, :, 1] = pad(b); demand[rule, i] = len(a) - 1
    covered, real = {}, {}
    for rule in (0, 2):
        tickets_, values, _ = lists[rule]
        covered[rule] = pad(walk(base, worse, tickets_, values, TAU_PROD, 0)[3])
        picked = [t for t, v in zip(tickets_, values) if v >= TAU_PROD]
        real[rule] = pad([0.0] + realised(g, picked, M))
    value.update(tables=tables, demand=demand, covered=covered, real=real, calibration=calibration(g, T, s3, base, M))
    return value


def journal_grids():
    """Dernière recommandation en vigueur de chaque grille du journal : entrées figées, tickets et espérances écrits par Rust."""
    path = os.path.expanduser("~/.local/share/edge/edge.db")
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    rows = db.execute("""select game, number, ticket, ev_pessimistic, inputs from lotofoot_reco r
                         where session_id = (select max(session_id) from lotofoot_reco x where x.game = r.game and x.year = r.year and x.number = r.number)
                         order by session_id desc, position""").fetchall()
    grids = {}
    for game, number, ticket, ev, inputs in rows:
        grids.setdefault((game, number), {"inputs": json.loads(inputs), "tickets": []})["tickets"].append((ticket, ev))
    return grids


def journal(key, item):
    game, inputs = int(key[0][2:]), item["inputs"]
    P, Q = np.array(inputs["p"]), np.array(inputs["q"]); S = inputs["stakes"]
    g = {"game": game, "S": S, "jackpot": {"type": 1, "amount": inputs.get("jackpot") or 0}}
    M = BF.masses(g); prm = PRM[game]
    T, Pt = tickets(P, Q, S, M, prm)
    base, worse = Exact(P, Q, S, M, prm, 1.0), Exact(Pt, Q, S, M, prm, LOW_MULT)
    gap = max(abs(worse.naive(["1N2".index(c) for c in ticket]) - ev) for ticket, ev in item["tickets"])
    s3 = [worse.naive(t) for t in T]
    order = order_by_s3(T, s3)
    T = [T[k] for k in order]; s3 = [s3[k] for k in order]
    v_s3, _, _, _ = walk(base, worse, T, s3, TAU_PROD, 0)
    return {"grid": f"{key[0]} n°{key[1]}", "gap_rust": gap, "above": len(v_s3) - 1, "s3": s3[:KMAX], "marginal": np.diff(v_s3).tolist(),
            "jackpot": inputs.get("jackpot"), "stakes": S}


def bucket_table(ev, label):
    print(f"\n{label} : espérance par ticket selon sa position (liste complète, sans seuil)")
    print("positions    grilles   naïve S3   marginale S3   naïve base   marginale base   part de la perte S3")
    for a, b in [(1, 8), (9, 16), (17, 24), (25, 32), (33, 48), (49, 64)]:
        s = np.mean([e["s3"][a - 1:b] for e in ev]); m = np.mean([e["marginal_s3"][a - 1:b] for e in ev])
        s_b = np.mean([e["naive_base"][a - 1:b] for e in ev]); m_b = np.mean([e["marginal_base"][a - 1:b] for e in ev])
        print(f"{a:2d}–{b:<2d}      {len(ev):5d}    {s:7.3f}    {m:7.3f}       {s_b:7.3f}       {m_b:7.3f}        {(s - m) / s:6.1%}")


def above_table(ev, label):
    i = TAUS.index(TAU_PROD)
    print(f"\n{label} : au seuil τ = {TAU_PROD}")
    print("tickets ≥ τ (naïf)   moyenne   P(> 16)   P(> 32)   P(= 64)   parmi les > 16 : marginale S3 moyenne   ≥ 1,7      ≥ 1")
    d = np.array([e["demand"][0, i] for e in ev])
    tail = [e["tables"][0, i, 17:e["demand"][0, i] + 1, 0] - e["tables"][0, i, 16:e["demand"][0, i], 0] for e in ev if e["demand"][0, i] > 16]
    flat = np.concatenate(tail) if tail else np.zeros(1)
    print(f"{'':20s} {d.mean():6.1f}   {np.mean(d > 16):6.1%}   {np.mean(d > 32):6.1%}   {np.mean(d >= 64):6.1%}"
          f"   {flat.mean():10.3f} ({len(flat)} tickets)            {np.mean(flat >= 1.7):6.1%}   {np.mean(flat >= 1):6.1%}")


def arrivals(mask, games, seed):
    rng = np.random.default_rng(seed)
    idx = {k: np.flatnonzero(mask & (games == k)) for k in (7, 8)}
    months = YEARS * 12
    counts = {k: rng.poisson(LAM[k], months) for k in (7, 8)}
    width = int((counts[7] + counts[8]).max())
    G = np.full((months, width), -1, dtype=np.int64)
    for m in range(months):
        row = np.concatenate([rng.choice(idx[7], counts[7][m]), rng.choice(idx[8], counts[8][m])])
        rng.shuffle(row); G[m, :len(row)] = row
    return G


def evaluate(data, G, rule, i, cap, budget):
    valid = G >= 0; gi = np.where(valid, G, 0)
    demand = np.where(valid, np.minimum(data["demand"][gi, rule, i], cap), 0)
    before = np.cumsum(demand, axis=1) - demand
    k = np.clip(budget - before, 0, demand)
    v = data["tables"][gi, rule, i, k, 0] - k
    b = data["tables"][gi, rule, i, k, 1] - k
    return {"rule": RULES[rule], "tau": TAUS[i], "cap": cap, "budget": budget, "spend": float(k.sum(axis=1).mean()),
            "s3_year": float((v * valid).sum() / YEARS), "base_year": float((b * valid).sum() / YEARS),
            "at_cap": float(((k >= cap) * valid).sum() / max(valid.sum(), 1))}


def sweep(data, mask, seed, label):
    G = arrivals(mask, data["game"], seed)
    rows, done, total = [], 0, len(SWEPT) * len(CAPS) * len(BUDGETS) * len(TAUS)
    for rule in SWEPT:
        for cap in CAPS:
            for budget in BUDGETS:
                rows += [evaluate(data, G, rule, i, cap, budget) for i in range(len(TAUS))]
                done += len(TAUS)
                log(f"{label} : {done} / {total} configurations")
    return rows


def show(best, title):
    print(f"\n{title}")
    print("règle       budget  plafond   τ    dépense/mois  rés. au plafond  V S3/an   écart/16   V base/an")
    for (rule, budget), line in best.items():
        ref = line[0]["s3_year"]
        for r in line:
            print(f"{rule:10s} {budget:4d} €   {r['cap']:3d}    {r['tau']:.1f}   {r['spend']:7.1f} €      {r['at_cap']:6.1%}     {r['s3_year']:+7.0f} €  {r['s3_year'] - ref:+6.0f} €  {r['base_year']:+7.0f} €")


def pick(rows, fixed=None):
    best = {}
    for rule in (RULES[k] for k in SWEPT):
        for budget in BUDGETS:
            line = []
            for cap in CAPS:
                subset = [r for r in rows if r["rule"] == rule and r["budget"] == budget and r["cap"] == cap]
                if fixed is None:
                    line.append(max(subset, key=lambda r: r["s3_year"]))
                else:
                    tau = next(r["tau"] for r in fixed[(rule, budget)] if r["cap"] == cap)
                    line.append(next(r for r in subset if r["tau"] == tau))
            best[(rule, budget)] = line
    return best


if __name__ == "__main__":
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    log(f"{len(grids)} grilles à évaluer, {KMAX} tickets par grille, {len(SWEPT) * len(CAPS) * len(BUDGETS) * len(TAUS)} configurations simulées")
    ev = []
    with Pool(8) as pool:
        for k, e in enumerate(pool.imap(work, grids, chunksize=2), 1):
            ev.append(e)
            if k % 10 == 0 or k == len(grids):
                rate = (time.time() - START) / k
                log(f"grilles {k} / {len(grids)}, reste environ {rate * (len(grids) - k):.0f} s")
    ref = np.load("../data/lf/budget_tau_cache.npz")
    gap = max(abs(a - b) for e, r in zip(ev, ref["s3"]) for a, b in zip(e["s3"][:HEAD], r))
    print(f"contrôle : écart maximal des 16 premières espérances S3 avec le §49 = {gap:.2e}")
    print(f"contrôle : écart maximal avec lf_fast.ev_p = {max(abs(e['ctrl_ev']) for e in ev):.2e}")
    data = {"game": np.array([e["game"] for e in ev]), "train": np.array([e["train"] for e in ev]),
            "tables": np.stack([e["tables"] for e in ev]), "demand": np.stack([e["demand"] for e in ev])}
    # `tables` et `demand` sont indexés grille, règle, τ… : `evaluate` les lit dans cet ordre.
    for game in (7, 8):
        sub = [e for e in ev if e["game"] == game]
        bucket_table(sub, f"LF{game}"); above_table(sub, f"LF{game}")
    i = TAUS.index(TAU_PROD)
    print("\nCouverture et gains réels au seuil de production (sans budget) :")
    print("règle    plafond   tickets par grille   P(grille payée), modèle   V S3 nette / grille   gains réels / €   IC 95 %   gain réel net")
    for rule in (0, 2):
        for cap in CAPS:
            k = np.minimum(data["demand"][:, rule, i], cap)
            rows = np.arange(len(ev))
            v = data["tables"][rows, rule, i, k, 0] - k
            real = np.array([e["real"][rule][c] for e, c in zip(ev, k)])
            ratio, low, high = ratio_ci(real, k.astype(float))
            print(f"{RULES[rule]:8s} {cap:4d}      {k.mean():6.2f}              {np.mean([e['covered'][rule][c] for e, c in zip(ev, k)]):6.1%}"
                  f"                   {v.mean():+7.2f} €        {ratio:6.2f}    [{low:.2f} ; {high:.2f}]   {real.sum() - k.sum():+8.0f} €")
    calibration_table(ev, "Toutes les grilles")
    calibration_table([e for e in ev if e["train"]], "Entraînement")
    calibration_table([e for e in ev if not e["train"]], "Test")
    log("gains réels calculés ; grilles du journal")
    items = journal_grids()
    with Pool(8) as pool:
        recent = pool.starmap(journal, list(items.items()))
    print("\nGrilles du journal (répartition de l'heure du calcul) :")
    print("grille        écart Rust   pactole     mises     tickets ≥ 1,7 (sur 64)   marginale S3 : 16e, 17e, 24e, 32e, dernier")
    for r in recent:
        m = r["marginal"]; at = lambda k: f"{m[k - 1]:.2f}" if len(m) >= k else "—"
        print(f"{r['grid']:12s}  {r['gap_rust']:.1e}   {r['jackpot'] or 0:8.0f}  {r['stakes']:8.0f}   {r['above']:3d}"
              f"                     {at(16)}  {at(17)}  {at(24)}  {at(32)}  {m[-1] if m else 0:.2f}")
    train, test = data["train"].astype(bool), ~data["train"].astype(bool)
    rows_train = sweep(data, train, 1, "entraînement")
    best = pick(rows_train)
    show(best, "Entraînement : τ* propre à chaque plafond (critère : V S3 par an, partage déduit)")
    rows_test = sweep(data, test, 2, "test")
    show(pick(rows_test, fixed=best), "Test, au τ* choisi sur l'entraînement")
    json.dump({"caps": CAPS, "budgets": BUDGETS, "train": rows_train, "test": rows_test, "recent": recent}, open(OUT, "w"))
    log(f"fini, résultats dans {OUT}")
