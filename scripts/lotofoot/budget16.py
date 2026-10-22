"""Budget de 16 € : pour chaque grille LF7/LF8 appariée, (a) 16 tickets simples gloutons μ = 10 distincts d'au moins 2 signes ;
(b) idem, distincts d'au moins 1 signe ; (c) meilleure grille multiple à 3 doubles (8 €) et à 4 doubles (16 €), cherchée par
descente (choix des matchs doublés et des signes). Espérances du modèle de base et du scénario S3 (concurrents × 1,4 si λ < 3,
p rapproché de q de 10 %). L'espérance d'une multiple est la somme de celles de ses combinaisons, puis corrigée du partage
entre nos propres combinaisons par simulation (nos tickets au même rang se partagent aussi la masse)."""
import sys, json, itertools, numpy as np
from multiprocessing import Pool
sys.path.insert(0, "."); import lf_fast
import backtest_fdj as BF
from frontier import PRM
MU = 10.0
def evp(t, P, Q, S, M, prm, lm=1.0):
    return lf_fast.ev_p(t, P, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=lm)
def greedy(P, Q, S, M, prm, K, dmin):
    # Réutilise la recherche de frontier.py (qui gère le cas sans départ admissible).
    from frontier import best_ticket
    n = P.shape[0]; picked = []
    starts = [[int(np.argmax(x)) for x in P], [int(np.argmax(P[i] / Q[i])) for i in range(n)]]
    for _ in range(K):
        t, _, _ = best_ticket(P, Q, S, M, prm, MU, picked, starts, dmin); picked.append(t)
    return picked
def multiple(P, Q, S, M, prm, nd):
    """Grille multiple : base (un signe par match) + nd matchs doublés (deux signes). Descente sur les choix."""
    n = P.shape[0]; cache = {}
    def combos(base, dbl):
        alts = [[base[i], dbl[i]] if dbl[i] is not None else [base[i]] for i in range(n)]
        return [list(c) for c in itertools.product(*alts)]
    def val(base, dbl):
        key = (tuple(base), tuple(dbl))
        if key not in cache:
            tot = 0.0
            for c in combos(base, dbl):
                e, pw = evp(c, P, Q, S, M, prm); tot += e + MU * pw
            cache[key] = tot
        return cache[key]
    base = [int(np.argmax(x)) for x in P]
    order = np.argsort([-sorted(x)[-2] for x in P])            # matchs les plus incertains d'abord
    dbl = [None] * n
    for i in order[:nd]: dbl[i] = int(np.argsort(-P[i])[1])
    cur = val(base, dbl)
    for _ in range(4):
        imp = False
        for i in range(n):
            for b in range(3):
                for d in ([None] + [x for x in range(3) if x != b]):
                    if (d is None) != (dbl[i] is None): continue          # garde le nombre de doubles
                    nb, ndb = list(base), list(dbl); nb[i], ndb[i] = b, d
                    v = val(nb, ndb)
                    if v > cur + 1e-12: base, dbl, cur, imp = nb, ndb, v, True
            for j in range(n):                                            # déplacer un double du match i au match j
                if dbl[i] is None or dbl[j] is not None: continue
                for d in range(3):
                    if d == base[j]: continue
                    nb, ndb = list(base), list(dbl); ndb[i], ndb[j] = None, d
                    v = val(nb, ndb)
                    if v > cur + 1e-12: base, dbl, cur, imp = nb, ndb, v, True
        if not imp: break
    return combos(base, dbl)
def realized_ev(tickets, P, Q, S, M, prm, shrink, lm, rng, NS=20000):
    """Espérance par simulation, partage entre nos propres tickets compris."""
    n = P.shape[0]; Pt = (1 - shrink) * P + shrink * Q; Pt /= Pt.sum(axis=1, keepdims=True); T = np.array(tickets); R = len(M)
    O = (rng.random((NS, n))[..., None] > np.cumsum(Pt, axis=1)[None, :, :-1]).sum(axis=2)
    a = Q[np.arange(n)[None, :], O]; dist = np.zeros((NS, n + 1)); dist[:, 0] = 1
    for i in range(n):
        x = a[:, i:i + 1]; dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    good = (T[None, :, :] == O[:, None, :]).sum(axis=2); gain = np.zeros(NS)
    for r in range(R):
        lam = S * dist[:, n - r]
        mu = prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
        mu = np.where(lam < 3, mu * lm, mu)
        c = (good == n - r).sum(axis=1)
        gain += np.where(c > 0, M[r] * c / (mu + c), 0.0)     # espérance de 1/(W + c) approchée par 1/(μ + c)
    return float(gain.mean()), float(np.mean(gain > 0))
def work(g):
    P, Q = np.array(g["p"]), np.array(g["q"]); S = g["S"]; M = BF.masses(g); prm = PRM[g["game"]]
    rng = np.random.default_rng(g["gid"])
    out = {"game": g["game"], "gid": g["gid"], "close": g["close"]}
    sets = {"simples d≥2 ×16": greedy(P, Q, S, M, prm, 16, 2), "simples d≥1 ×16": greedy(P, Q, S, M, prm, 16, 1),
            "multiple 3 doubles (8 €)": multiple(P, Q, S, M, prm, 3), "multiple 4 doubles (16 €)": multiple(P, Q, S, M, prm, 4)}
    for name, T in sets.items():
        out[name] = {sc: realized_ev(T, P, Q, S, M, prm, sh, lm, rng) for sc, sh, lm in (("base", 0.0, 1.0), ("S3", 0.1, 1.4))}
        out[name]["cost"] = len(T)
    # espérances individuelles (S3) des 16 simples d≥2, pour la règle de seuil
    Pt = 0.9 * P + 0.1 * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    out["ev_ind_S3"] = [lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=1.4)[0] for t in sets["simples d≥2 ×16"]]
    out["ev_ind_base"] = [evp(t, P, Q, S, M, prm)[0] for t in sets["simples d≥2 ×16"]]
    return out
if __name__ == "__main__":
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    with Pool(8) as pool: res = pool.map(work, grids, chunksize=2)
    json.dump(res, open("../data/lf/budget16.json", "w"))
    for name in ("simples d≥2 ×16", "simples d≥1 ×16", "multiple 3 doubles (8 €)", "multiple 4 doubles (16 €)"):
        for game in (7, 8):
            sub = [r for r in res if r["game"] == game]; c = sub[0][name]["cost"]
            print(f"LF{game} {name:26s}: EV/€ base {np.mean([r[name]['base'][0] for r in sub])/c:.3f}, S3 {np.mean([r[name]['S3'][0] for r in sub])/c:.3f}, "
                  f"P(grille payée) {np.mean([r[name]['base'][1] for r in sub]):.1%}")
