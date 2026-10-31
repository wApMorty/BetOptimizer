"""Intervalle du gain d'espérance du modèle de revenu net sur la production, à 100 et 200 € par mois (2026-10-31).

Gain = revenu net S3 par an du modèle (« copies », sans plafond, prix dynamique, ρ = 1, DP de l'entraînement) − production (vivier, τ de
l'abaque, 16 par grille), entrées bruitées. L'incertitude vient surtout du petit lot de grilles : 500 tirages avec remise du lot, comparaison
appariée sur 4 000 mois (les arrivées de `net_revenue.arrivals` sont réindexées sur le lot tiré). Ne mesure que l'espérance du modèle.

Usage, depuis scripts/lotofoot : `~/.local/share/edge/etudes/venv/bin/python net_ci.py > ../data/lf/net_ci.txt 2> ../data/lf/net_ci.log`
"""
import sys, time, datetime as dt, numpy as np
from multiprocessing import get_context
sys.path.insert(0, ".")
import net_revenue as N, budget_tau as BT

MONTHS, DRAWS = 4000, 500
SCALE = BT.YEARS * 12 / MONTHS
KEY = "noisy_copies"
state = {}


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - N.START:6.0f} s] {message}", file=sys.stderr, flush=True)


def one(args):
    seed, budget = args
    data, pool, G, steps, W = state["data"], state["pool"], state["G"], state["steps"], state["W"][budget]
    rng = np.random.default_rng([seed, budget])
    mapping = np.arange(len(data["game"]))
    for game in (7, 8):
        ids = pool[data["game"][pool] == game]
        mapping[ids] = rng.choice(ids, len(ids))
    picked = np.where(G >= 0, mapping[np.where(G >= 0, G, 0)], -1)
    model = N.net_dynamic(data, KEY, picked, steps, W, 1.0, budget, 96)[0] * SCALE
    prod = N.net_static(data, "noisy_prod", picked, N.ABAQUE[budget], budget, 16)[0] * SCALE
    return model, prod


if __name__ == "__main__":
    z = np.load(N.CACHE); data = {k: z[k] for k in z.files}
    data["game"] = data["game"].astype(np.int64)
    train = data["train"].astype(bool)
    state["data"] = data
    state["W"] = {}
    for budget in (100, 200):
        est = data[f"est_{KEY}"][train]
        curves = np.concatenate([np.zeros((len(est), 1)), np.cumsum(est, axis=1)], axis=1) - np.arange(N.K + 1)
        state["W"][budget] = N.solve_dp(curves, N.weights_of(data, train), budget, 1.0)
        log(f"programme dynamique résolu, {budget} €")
    print("lot  budget  production  modèle  gain  IC 95 % du gain")
    for label, mask, seed in (("entraînement", train, 1), ("test", ~train, 2)):
        G, _, steps = N.arrivals(data, mask, seed)
        state.update(pool=np.flatnonzero(mask), G=G[:MONTHS], steps=steps[:MONTHS])
        for budget in (100, 200):
            with get_context("fork").Pool(6) as pool:      # « fork » : les processus héritent de `state` sans le recopier
                out = []
                for k, r in enumerate(pool.imap(one, [(s, budget) for s in range(DRAWS)], chunksize=5), 1):
                    out.append(r)
                    if k % 100 == 0:
                        log(f"{label} {budget} € : {k} / {DRAWS}, reste environ {(time.time() - N.START) / k * (DRAWS - k):.0f} s")
            a = np.array(out); gain = a[:, 0] - a[:, 1]
            print(f"{label:12s} {budget:3d} €  {a[:, 1].mean():+7.0f} €  {a[:, 0].mean():+7.0f} €  {gain.mean():+6.0f} €  "
                  f"[{np.percentile(gain, 2.5):+.0f} ; {np.percentile(gain, 97.5):+.0f}]  P(gain > 0) {np.mean(gain > 0):.3f}", flush=True)
    log("fini")
