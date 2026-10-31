"""Contrôle du modèle de revenu net sur les résultats réels : que rapportent les suites de tickets, sans budget ? (2026-10-31)

Pour chaque règle (suite, seuil τ, plafond), sur chaque grille : gain = Σ_r M_r · c_r / (W_r + c_r), c_r nos tickets au rang r pour
l'issue réelle, W_r les gagnants publiés. Les mises étant inégales d'une grille à l'autre, on rapporte les gains par euro et un intervalle
à 95 % (bootstrap de grilles, 5 000 tirages). Espérés : mêmes rangs en base et en S3. Un sans-faute jamais observé n'est pas vu par un
bootstrap : l'intervalle est un minimum d'incertitude, pas sa mesure. Argument : `train` (lit l'entraînement) ou `test` (lit le test).

Usage, depuis scripts/lotofoot : `~/.local/share/edge/etudes/venv/bin/python net_real.py train`
"""
import sys, json, numpy as np
sys.path.insert(0, ".")
import backtest_fdj as BF, net_core as C

RULES = [("prod τ 1,7", "noisy_prod", 1.7, 16), ("exact τ 1,7", "noisy_exact", 1.7, 16), ("copies τ 1,7", "noisy_copies", 1.7, 16),
         ("copies τ 1,7 sans plafond", "noisy_copies", 1.7, 96), ("copies τ 2,5 sans plafond", "noisy_copies", 2.5, 96)]


def demand(est, tau, cap):
    below = est < tau
    return min(int(below.argmax()) if below.any() else len(est), cap)


if __name__ == "__main__":
    split = sys.argv[1]
    grids = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    z = np.load("../data/lf/net_revenue_cache.npz")
    want = (z["train"].astype(bool) if split == "train" else ~z["train"].astype(bool))
    print(f"{split} : règle  grilles jouées  mises  gains  gains par €  IC 95 %  sans-faute (base/S3/réel)  une erreur (base/S3/réel)")
    for label, key, tau, cap in RULES:
        stakes, gains, rows = [], [], np.zeros(6)
        for i, g in enumerate(grids):
            if not want[i]:
                continue
            k = demand(z[f"est_{key}"][i], tau, cap)
            if k == 0:
                stakes.append(0); gains.append(0.0)
                continue
            seq = z[f"seq_{key}"][i][:k]
            n = g["n"]; o = C.code_of(g["o"]); M = BF.masses(g)
            near = set(C.neighbours(n)[o].tolist())
            c0 = int(np.sum(seq == o)); c1 = int(sum(int(t) in near for t in seq))
            W = {n - r["good"]: r["W"] for r in g["ranks"]}
            gain = 0.0
            if c0 and 0 in W: gain += M[0] * c0 / (W[0] + c0)
            if c1 and 1 in W: gain += M[1] * c1 / (W[1] + c1)
            stakes.append(k); gains.append(gain)
            P, Q = np.array(g["p"]), np.array(g["q"])
            for j, table_p in enumerate((P, C.s3_probabilities(P, Q))):
                prob = np.prod(table_p[np.arange(n)[None, :], C.codes(n)], axis=1)
                rows[j] += float(prob[seq].sum())
                rows[3 + j] += float(prob[C.neighbours(n)[seq]].sum())
            rows[2] += c0; rows[5] += c1
        stakes, gains = np.array(stakes, float), np.array(gains)
        rng = np.random.default_rng(7)
        picks = rng.integers(0, len(gains), (5000, len(gains)))
        ratios = gains[picks].sum(axis=1) / np.maximum(stakes[picks].sum(axis=1), 1)
        # Loi du nombre de rangs atteints sous S3 : simulation des issues, grille par grille, mêmes tickets (les copies tombent ensemble).
        sim = np.zeros(4000); sim0 = np.zeros(4000)
        rng = np.random.default_rng(11)
        for i, g in enumerate(grids):
            if not want[i]:
                continue
            k = demand(z[f"est_{key}"][i], tau, cap)
            if k == 0:
                continue
            P, Q = np.array(g["p"]), np.array(g["q"]); Pt = C.s3_probabilities(P, Q); n = g["n"]
            O = (rng.random((4000, n))[..., None] > np.cumsum(Pt, axis=1)[None, :, :-1]).sum(axis=2)
            T = C.codes(n)[z[f"seq_{key}"][i][:k]]
            good = (T[None, :, :] == O[:, None, :]).sum(axis=2)
            sim += (good == n - 1).sum(axis=1); sim0 += (good == n).sum(axis=1)
        print(f"   une erreur sous S3 : moyenne {sim.mean():.1f}, IC 95 % [{np.percentile(sim, 2.5):.0f} ; {np.percentile(sim, 97.5):.0f}], P(≤ réel) {np.mean(sim <= rows[5]):.3f} ; sans-faute : P(≤ réel) {np.mean(sim0 <= rows[2]):.3f}")
        print(f"{label:28s} {int((stakes > 0).sum()):4d}  {stakes.sum():6.0f} €  {gains.sum():8.0f} €  {gains.sum() / max(stakes.sum(), 1):5.2f}  "
              f"[{np.percentile(ratios, 2.5):.2f} ; {np.percentile(ratios, 97.5):.2f}]  {rows[0]:.2f} / {rows[1]:.2f} / {rows[2]:.0f}   {rows[3]:.1f} / {rows[4]:.1f} / {rows[5]:.0f}")
