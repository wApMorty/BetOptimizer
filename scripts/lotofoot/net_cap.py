"""Phase 20, étape 0 : quel plafond de tickets par grille pour la règle exacte à prix dynamique et gains recyclés ? (2026-10-31)

Protocole, figé avant le calcul ; étude descriptive, le choix revient à Paul :
- règle : suite « exact » de `net_revenue.py` (sans copie, entrées bruitées), prix dynamique, ρ = 1, un programme dynamique par plafond,
  résolu sur l'entraînement seul ; production : vivier, τ de l'abaque, 16 par grille ;
- plafonds : 16, 24, 32, 48, 64, 96 ; budgets : 100 et 200 € par mois ; budget brut et recyclé ; entraînement et test ;
- vérité : S3, tickets évalués sur q et S vrais, partage exact ; loi annuelle par 1 000 tirages par grille (`net_revenue.draw_work`),
  4 000 années, public juste à côté ;
- intervalle du gain d'espérance sur la production, à modes égaux : 300 tirages avec remise du lot de grilles, appariés, sur 3 000 mois
  (plafonds 16, 32, 48 et 96) ;
- charge au guichet : grilles à remplir pour k tickets, regroupement glouton (la plus grande grille autorisée d'abord) sur 40 grilles de
  test, interpolée sur les tickets joués.
Configurations : 6 plafonds × 2 budgets × 2 modes + production, sans choix automatique. Limites : celles du §69.

Usage, depuis scripts/lotofoot, en arrière-plan :
  ~/.local/share/edge/etudes/venv/bin/python net_cap.py > ../data/lf/net_cap.txt 2> ../data/lf/net_cap.log &
"""
import sys, json, time, itertools, datetime as dt, numpy as np
from multiprocessing import Pool, get_context
sys.path.insert(0, ".")
import net_revenue as N, budget_tau as BT, net_core as C

CAPS = [16, 24, 32, 48, 64, 96]
CI_CAPS = [16, 32, 48, 96]
BUDGETS = [100, 200]
MONTHS, DRAWS = 3000, 300
SCALE = BT.YEARS * 12 / MONTHS
KEY, PROD = "noisy_exact", "noisy_prod"
SIZES = [1, 2, 4, 8, 16, 24, 32, 48, 64, 96]
MAX_DOUBLES = {7: [4, 3, 1, 0], 8: [5, 3, 2, 0]}
state = {}


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - N.START:6.0f} s] {message}", file=sys.stderr, flush=True)


def allowed(game, line):
    doubles = sum(len(x) == 2 for x in line); triples = sum(len(x) == 3 for x in line)
    return triples < 4 and doubles <= MAX_DOUBLES[game][triples]


def lines_needed(game, tickets):
    """Grilles à remplir : tant qu'il reste des tickets, la plus grande grille autorisée formée uniquement de ces tickets."""
    left, count = set(map(tuple, tickets)), 0
    while left:
        seen = {tuple((s,) for s in t) for t in left}
        frontier = list(seen)
        while frontier:
            found = []
            for line in frontier:
                for i, signs in enumerate(line):
                    for sign in set(range(3)) - set(signs):
                        wider = line[:i] + (tuple(sorted(signs + (sign,))),) + line[i + 1:]
                        if wider not in seen and allowed(game, wider) and all(x in left for x in itertools.product(*wider)):
                            seen.add(wider); found.append(wider)
            frontier = found
        best = max(seen, key=lambda line: (int(np.prod([len(x) for x in line])), line))
        left -= set(itertools.product(*best))
        count += 1
    return count


def lines_item(item):
    game, seq = item
    return [lines_needed(game, C.codes(game)[seq[:k]].tolist()) for k in SIZES]


def lines_curve(data):
    """Grilles à remplir moyennes pour k tickets, k = 0…96."""
    ids = np.flatnonzero(~data["train"].astype(bool))[:40]
    with Pool(8) as pool:
        rows = np.array(pool.map(lines_item, [(int(data["game"][i]), data[f"seq_{KEY}"][i]) for i in ids]), float)
    log("charge au guichet mesurée")
    return np.interp(np.arange(N.K + 1), [0] + SIZES, [0.0] + rows.mean(axis=0).tolist()), rows.mean(axis=0)


def table(data, train, cap):
    est = data[f"est_{KEY}"][train][:, :cap]
    curves = np.concatenate([np.zeros((len(est), 1)), np.cumsum(est, axis=1)], axis=1) - np.arange(cap + 1)
    return N.solve_dp(curves, N.weights_of(data, train), 200, 2.0)


def ev_of(data, key, G, Ks):
    valid = G >= 0; gi = np.where(valid, G, 0)
    return np.where(valid, data[f"cum_{key}_s3"][gi, Ks] - Ks, 0.0).sum()


def line(label, data, key, G, Ks, gains, null_gains, curve):
    net = (gains - Ks).reshape(BT.YEARS, -1).sum(axis=1)
    worst = (null_gains - Ks).reshape(BT.YEARS, -1).sum(axis=1)
    played = Ks[Ks > 0]
    print(f"{label:28s} {ev_of(data, key, G, Ks) / BT.YEARS:+7.0f} €  {net.mean():+7.0f} €  {net.std():6.0f} €  {np.median(net):+6.0f} €  {np.mean(net < 0):5.1%}"
          f"  {(gains > 0).reshape(BT.YEARS, -1).sum(axis=1).mean():5.2f}  {(gains > 0).any(axis=1).mean():5.1%}  {Ks.sum(axis=1).mean():6.1f} €  {(Ks > 0).sum(axis=1).mean():5.1f}"
          f"  {played.mean():5.1f}  {np.percentile(played, 90):4.0f}  {curve[Ks].sum(axis=1).mean():6.1f}  {worst.mean():+7.0f} €", flush=True)


def one(args):
    seed, budget, recycled = args
    data, pool, G, D, steps = state["data"], state["pool"], state["G"], state["D"], state["steps"]
    rng = np.random.default_rng([seed, budget, recycled])
    mapping = np.arange(len(data["game"]))
    for game in (7, 8):
        ids = pool[data["game"][pool] == game]
        mapping[ids] = rng.choice(ids, len(ids))
    picked = np.where(G >= 0, mapping[np.where(G >= 0, G, 0)], -1)
    Ks, _ = N.simulate(data, PROD, state["sims"][PROD][0], picked, D, steps, budget, recycled, N.ABAQUE[budget], 16, None)
    base = ev_of(data, PROD, picked, Ks)
    out = []
    for cap in CI_CAPS:
        Ks, _ = N.simulate(data, KEY, state["sims"][KEY][0], picked, D, steps, budget, recycled, None, cap, state["W"][cap])
        out.append((ev_of(data, KEY, picked, Ks) - base) * SCALE / BT.YEARS)
    return out


if __name__ == "__main__":
    z = np.load(N.CACHE); data = {k: z[k] for k in z.files}
    data["game"] = data["game"].astype(np.int64)
    train = data["train"].astype(bool)
    curve, sizes = lines_curve(data)
    print("Grilles à remplir pour k tickets (40 grilles de test) : " + "  ".join(f"{k} : {v:.1f}" for k, v in zip(SIZES, sizes)))
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    names = [PROD, KEY]
    with Pool(8) as pool:
        done = pool.map(N.draw_work, [(g, {n: data[f"seq_{n}"][i] for n in names}) for i, g in enumerate(grids)], chunksize=4)
    sims = {n: (np.stack([d[n][0] for d in done]), np.stack([d[n][1] for d in done])) for n in names}
    del done
    log("tirages faits")
    W = {}
    for k, cap in enumerate(CAPS, 1):
        W[cap] = table(data, train, cap)
        log(f"programme dynamique {k} / {len(CAPS)}, plafond {cap}")
    state.update(data=data, sims=sims, W=W)
    head = "règle                         EV S3/an  simulé/an  écart type  médiane  P(perte)  payées  mois+  mises/mois  grilles/mois  t./grille  p90  à remplir/mois  public juste"
    for label, mask, seed in (("Entraînement", train, 1), ("Test", ~train, 2)):
        G, D, steps = N.arrivals(data, mask, seed)
        for budget in BUDGETS:
            print(f"\n{label}, {budget} € par mois\n{head}", flush=True)
            for recycled in (False, True):
                tag = "recyclé" if recycled else "brut"
                K_, g_ = N.simulate(data, PROD, sims[PROD][0], G, D, steps, budget, recycled, N.ABAQUE[budget], 16, None)
                _, n_ = N.simulate(data, PROD, sims[PROD][1], G, D, steps, budget, recycled, N.ABAQUE[budget], 16, None)
                line(f"production {tag}", data, PROD, G, K_, g_, n_, curve)
                for cap in CAPS:
                    K_, g_ = N.simulate(data, KEY, sims[KEY][0], G, D, steps, budget, recycled, None, cap, W[cap])
                    _, n_ = N.simulate(data, KEY, sims[KEY][1], G, D, steps, budget, recycled, None, cap, W[cap])
                    line(f"exact dyn. {cap} {tag}", data, KEY, G, K_, g_, n_, curve)
                    log(f"{label} {budget} € {tag} plafond {cap}")
    print("\nGain d'espérance S3 sur la production par an, mêmes budget et mode ; IC 95 %, 300 tirages du lot de grilles")
    print("lot  budget  mode  " + "  ".join(f"plafond {c}" for c in CI_CAPS), flush=True)
    for label, mask, seed in (("entraînement", train, 1), ("test", ~train, 2)):
        G, D, steps = N.arrivals(data, mask, seed)
        state.update(pool=np.flatnonzero(mask), G=G[:MONTHS], D=D[:MONTHS], steps=steps[:MONTHS])
        for budget in BUDGETS:
            for recycled in (False, True):
                begin = time.time(); out = []
                with get_context("fork").Pool(6) as pool:      # « fork » : les processus héritent de `state` sans le recopier
                    for k, r in enumerate(pool.imap(one, [(s, budget, recycled) for s in range(DRAWS)], chunksize=5), 1):
                        out.append(r)
                        if k % 50 == 0:
                            log(f"{label} {budget} € recyclé={recycled} : {k} / {DRAWS}, reste environ {(time.time() - begin) / k * (DRAWS - k):.0f} s")
                a = np.array(out)
                cells = "  ".join(f"{a[:, j].mean():+5.0f} [{np.percentile(a[:, j], 2.5):+.0f} ; {np.percentile(a[:, j], 97.5):+.0f}]" for j in range(len(CI_CAPS)))
                print(f"{label:12s} {budget:3d} €  {'recyclé' if recycled else 'brut':7s} {cells}", flush=True)
    log("fini")
