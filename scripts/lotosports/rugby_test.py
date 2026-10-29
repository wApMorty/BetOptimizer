"""Loto Rugby 7 : lecture unique du test (grilles closes à partir du 2024-01-01), protocole PROTOCOLE_RUGBY.md.

Paramètres gelés lus dans `params_rugby.json` ; le script refuse de tourner si leur empreinte SHA-256 diffère de celle consignée
dans `docs/sources.md` (commit c4578bc). Mesures et verdict exactement ceux du protocole :
  - ROI réalisé, IC 95 % par bootstrap des grilles (10 000 tirages, graine 20261029), ROI sans le meilleur gain, grilles payées ;
  - espérance annoncée ; calibration des concurrents par tranche de λ ;
  - loi simulée du gain sous le modèle (20 000 tirages) ; hypothèse nulle « tickets du public » (10 000 jeux de 10 tickets selon q) ;
  - analyses secondaires : logistique, 1 et 3 tickets, S réel.

Usage : python3 -I rugby_test.py DOSSIER_LS   (venv des études : numpy, scipy)
Journal horodaté sur stderr.
"""
import hashlib, json, os, sys
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rugby_fit as R  # noqa: E402

FROZEN_SHA = "f882da39d0f6de5eec3ed8710a3681769022705bc83400e3f79577ce4f184dcd"
SEED = 20261029
N_BOOT, N_SIM, N_NULL = 10_000, 20_000, 10_000


def roi(gain, cost):
    return gain.sum() / cost.sum() - 1


def bootstrap(rows, rng):
    gain = np.array([r["gain"] for r in rows])
    cost = np.array([r["cost"] for r in rows])
    idx = rng.integers(0, len(rows), size=(N_BOOT, len(rows)))
    b = gain[idx].sum(axis=1) / cost[idx].sum(axis=1) - 1
    return np.percentile(b, [2.5, 97.5])


def hit_tables(tickets):
    """Pour chaque issue (3⁷) : nombre de nos tickets à 7 bons et à 6 bons."""
    good = (R.OUT[:, None, :] == tickets[None, :, :]).sum(axis=2)
    return (good == R.N).sum(axis=1), (good == R.N - 1).sum(axis=1)


def nb_draw(rng, m, th):
    """Binomiale négative de moyenne m et de taille θ, par mélange gamma-Poisson."""
    return rng.poisson(rng.gamma(th, m / th))


def simulate(test, rows, prm, rng):
    """Loi du gain total sous le modèle : issues tirées selon p, concurrents selon la binomiale négative gelée, S réel."""
    tot = np.zeros(N_SIM)
    for g, r in zip(test, rows):
        prob, l1, l2 = R.outcome_table(r["P"], g["q"], g["S"])
        c1, c2 = hit_tables(r["tickets"])
        o = rng.choice(len(R.OUT), size=N_SIM, p=prob / prob.sum())
        W1 = nb_draw(rng, R.mu(l1[o], g["S"], prm), prm["theta"])
        W2 = nb_draw(rng, R.mu(l2[o], g["S"], prm), prm["theta"])
        tot += R.payout(c1[o], c2[o], W1, W2, g["S"] + r["cost"], g["jackpot"])
    return tot


def null_public(test, k, rng):
    """10 000 jeux de k tickets tirés signe par signe selon q, gains avec les gagnants publiés."""
    tot = np.zeros(N_NULL)
    for g in test:
        cq = np.cumsum(g["q"], axis=1)
        u = rng.random((N_NULL, k, R.N))
        T = (u[:, :, :, None] > cq[None, None, :, :-1]).sum(axis=3)       # signe 0, 1 ou 2 selon q
        good = (T == g["o"][None, None, :]).sum(axis=2)
        c1, c2 = (good == R.N).sum(axis=1), (good == R.N - 1).sum(axis=1)
        tot += R.payout(c1, c2, g["W"][0], g["W"][1], g["S"] + k, g["jackpot"])
    return tot


def expected_paid(test, rows):
    """Nombre attendu de grilles payées : Σ P(au moins un ticket à 0 ou 1 erreur) sous p."""
    e = 0.0
    for g, r in zip(test, rows):
        prob = R.outcome_table(r["P"], g["q"], g["S"])[0]
        c1, c2 = hit_tables(r["tickets"])
        e += prob[(c1 + c2) > 0].sum()
    return e


def summary(name, rows, rng):
    gain = np.array([r["gain"] for r in rows])
    cost = np.array([r["cost"] for r in rows])
    lo, hi = bootstrap(rows, rng)
    best = gain.argmax()
    no_best = (gain.sum() - gain[best]) / cost.sum() - 1
    print(f"{name} : {len(rows)} grilles, {cost.sum():.0f} € misés, {gain.sum():.2f} € gagnés, ROI {roi(gain, cost):+.1%} "
          f"[IC 95 % {lo:+.1%} ; {hi:+.1%}] ; sans le meilleur gain {no_best:+.1%} ; espérance annoncée "
          f"{sum(r['ev'] for r in rows) / cost.sum():.3f} € par euro ; grilles payées {int((gain > 0).sum())} ; "
          f"1er rang {sum(r['c1'] for r in rows)}, 2e rang {sum(r['c2'] for r in rows)}")
    return gain.sum(), lo


def main():
    root = sys.argv[1]
    path = os.path.join(root, "params_rugby.json")
    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
    if digest != FROZEN_SHA:
        sys.exit(f"empreinte de {path} différente de celle consignée ({digest}) : test refusé")
    prm = json.load(open(path, encoding="utf-8"))
    grids, stakes = R.load(root)
    test = [g for g in grids if g["close"] >= prm["cutoff"]]
    R.log(f"paramètres gelés vérifiés ; {len(test)} grilles de test")

    rows = R.strategy(test, stakes, prm["train_median_S"], prm, 10)
    print("=== STRATÉGIE PRINCIPALE (10 tickets, modèle sans cote, Ŝ) ===")
    real, lo = summary("TEST", rows, np.random.default_rng(SEED))
    print(f"  grilles payées attendues sous le modèle : {expected_paid(test, rows):.1f}")
    for r in rows:
        if r["gain"] > 0:
            print(f"  n°{r['num']} ({r['close'][:10]}) : {r['c1']} au 1er rang, {r['c2']} au 2e, gain {r['gain']:.2f} €")

    flags = R.calib_table(R.observations(test), prm, "CALIBRATION DES CONCURRENTS, TEST (paramètres gelés)")
    R.log(f"loi simulée du gain ({N_SIM} tirages)")
    sim = simulate(test, rows, prm, np.random.default_rng([SEED, 1]))
    p5, med, p95 = np.percentile(sim, [5, 50, 95])
    print(f"LOI SIMULÉE du gain total : moyenne {sim.mean():.0f} €, 5e centile {p5:.0f}, médiane {med:.0f}, 95e centile {p95:.0f} ; "
          f"réalisé {real:.0f} € ; P(simulé ≤ réalisé) = {np.mean(sim <= real):.3f}")
    cost = sum(r["cost"] for r in rows)
    R.log(f"hypothèse nulle : {N_NULL} jeux de tickets du public")
    null = null_public(test, 10, np.random.default_rng([SEED, 2]))
    pval = float(np.mean(null >= real))
    print(f"TICKETS DU PUBLIC (10 par grille) : gain moyen {null.mean():.0f} € (ROI {null.mean() / cost - 1:+.1%}), "
          f"95e centile {np.percentile(null, 95):.0f} ; P(public ≥ réalisé) = {pval:.4f}")

    bad_calib = [f for f in flags if f[1] >= 10 and not 0.8 <= f[2] <= 1.25]
    defect = bool(bad_calib) or real < p5
    roi_main = real / cost - 1
    if lo > 0:
        verdict = "AVANTAGE DÉMONTRÉ"
    elif roi_main > 0 and pval < 0.05 and not defect:
        verdict = "AVANTAGE PROBABLE"
    else:
        verdict = "NON DÉMONTRÉ"
    print(f"=== VERDICT : {verdict} ; modèle en défaut : {'oui' if defect else 'non'}"
          f" (tranches hors bornes : {bad_calib or 'aucune'} ; réalisé sous le 5e centile : {'oui' if real < p5 else 'non'}) ===")

    print("=== ANALYSES SECONDAIRES (sans rôle dans le verdict) ===")
    rng = np.random.default_rng([SEED, 3])
    summary("1. logistique, 10 tickets", R.strategy(test, stakes, prm["train_median_S"], prm, 10, model="logit"), rng)
    summary("2a. 1 ticket", R.strategy(test, stakes, prm["train_median_S"], prm, 1), rng)
    summary("2b. 3 tickets", R.strategy(test, stakes, prm["train_median_S"], prm, 3), rng)
    summary("3. S réel, 10 tickets", R.strategy(test, stakes, prm["train_median_S"], prm, 10, use_real_s=True), rng)
    R.log("fini")


if __name__ == "__main__":
    main()
