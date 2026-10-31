"""Phase 20 : table gelée des seuils du prix dynamique, plafond de 32 tickets par grille (`crates/core/data/lotofoot_price.csv`, §70).

Seuil de valeur marginale d'un ticket : 1 + W_t(b) − W_t(b − 1), W résolu par `net_revenue.solve_dp` sur les 319 grilles d'entraînement
(suite « exact », entrées bruitées, ρ = 1), t pas restants du mois (120 pas), b euros restants. W ne dépend pas du budget mensuel : une
seule table sert à tous les budgets. Lignes : t = 0, 5…120 (interpolées en t) ; colonnes : b = 1…200, puis de 10 en 10 jusqu'à 800 (palier
inférieur). Contrôle : le revenu net S3 avec la table réduite contre la table complète, entraînement et test.

Usage, depuis scripts/lotofoot : `~/.local/share/edge/etudes/venv/bin/python net_price.py`
"""
import sys, numpy as np
sys.path.insert(0, ".")
import net_revenue as N, budget_tau as BT

CAP, KEY = 32, "noisy_exact"
OUT = "../../crates/core/data/lotofoot_price.csv"
T_NODES = np.arange(0, N.STEPS + 1, 5)
B_NODES = np.unique(np.concatenate([np.arange(1, 201), np.arange(200, 801, 10)]))


def main():
    z = np.load(N.CACHE); data = {k: z[k] for k in z.files}
    data["game"] = data["game"].astype(np.int64); train = data["train"].astype(bool)
    est = data[f"est_{KEY}"][train][:, :CAP]
    curves = np.concatenate([np.zeros((len(est), 1)), np.cumsum(est, axis=1)], axis=1) - np.arange(CAP + 1)
    N.log("résolution du programme dynamique")
    W = N.solve_dp(curves, N.weights_of(data, train), 400, 2.0)
    bar = 1 + np.diff(W, axis=1)
    table = np.round(bar[np.ix_(T_NODES, B_NODES - 1)], 4)
    assert (np.diff(W, 2, axis=1) <= 1e-9).all() and (bar >= 1 - 1e-9).all() and (np.diff(table, axis=0) >= -1e-3).all()
    with open(OUT, "w") as f:
        f.write("# Seuils du prix dynamique Loto Foot, plafond de 32 tickets par grille : généré par scripts/lotofoot/net_price.py, ne pas modifier (§70).\n")
        f.write("t," + ",".join(map(str, B_NODES)) + "\n")
        for t, row in zip(T_NODES, table):
            f.write(f"{t}," + ",".join(f"{x:.4f}" for x in row) + "\n")
    N.log(f"{table.size} valeurs écrites dans {OUT}")

    def reduced(t, b):
        low = np.minimum(t // 5, len(T_NODES) - 2); w = (t - T_NODES[low]) / 5.0
        col = np.searchsorted(B_NODES, b, side="right") - 1
        return (1 - w) * table[low, col] + w * table[low + 1, col]

    def full(t, b):
        return bar[t, np.minimum(b, bar.shape[1]) - 1]

    def run(G, steps, budget, price):
        e = data[f"est_{KEY}"][:, :CAP]; cum = data[f"cum_{KEY}_s3"]
        valid = G >= 0; gi = np.where(valid, G, 0)
        left = np.full(G.shape[0], budget, dtype=np.int64); net = 0.0
        for a in range(G.shape[1]):
            live = np.flatnonzero(valid[:, a])
            if not len(live):
                break
            b = left[live][:, None] - np.arange(CAP)
            ok = (e[gi[live, a]] >= price(steps[live, a][:, None] - 1, np.maximum(b, 1))) & (b >= 1)
            k = np.where(ok.all(axis=1), CAP, (~ok).argmax(axis=1))
            net += float((cum[gi[live, a], k] - k).sum()); left[live] -= k
        return net * 12 / G.shape[0]

    for label, mask, seed in (("entraînement", train, 1), ("test", ~train, 2)):
        G, _, steps = N.arrivals(data, mask, seed)
        G, steps = G[:8000], steps[:8000]
        for budget in (60, 100, 200, 400):
            N.log(f"{label} {budget} € : table complète {run(G, steps, budget, full):+.0f} €, table réduite {run(G, steps, budget, reduced):+.0f} € par an")


if __name__ == "__main__":
    main()
