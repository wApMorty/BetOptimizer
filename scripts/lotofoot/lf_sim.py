"""Simulation d'une grille : issues tirées selon le marché, gains de nos tickets selon le modèle calibré du public
(concurrents ~ Poisson(κ λ^β) avec un facteur de grille log-normal σ). Renvoie la loi du gain total de la grille."""
import numpy as np
def simulate(tickets, P, Q, S, masses, n_sims, rng, kappa=1.15, beta=0.976, sigma=0.6):
    n, R = P.shape[0], len(masses); T = np.asarray(tickets)
    cum = np.cumsum(P, axis=1)
    U = rng.random((n_sims, n)); O = (U[:, :, None] > cum[None, :, :-1]).sum(axis=2)          # issues tirées
    a = Q[np.arange(n)[None, :], O]
    dist = np.zeros((n_sims, n + 1)); dist[:, 0] = 1.0
    for i in range(n):
        x = a[:, i:i + 1]; dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    eps = np.exp(sigma * rng.standard_normal(n_sims) - sigma ** 2 / 2)
    gain = np.zeros(n_sims)
    good = (T[None, :, :] == O[:, None, :]).sum(axis=2)                 # bons pronostics de chaque ticket
    for r in range(R):
        lam = kappa * np.power(np.maximum(S * dist[:, n - r], 1e-300), beta) * eps
        W = rng.poisson(lam)                                             # concurrents au rang r+1
        ours = (good == n - r).sum(axis=1)                               # nos tickets à ce rang
        gain += np.where(ours > 0, masses[r] * ours / (W + ours), 0.0)
    return gain
