"""Espérance vectorisée d'un ticket simple (même modèle que lf_ev.ev_corrected, sans bruit) : toutes les issues à moins de
R erreurs sont énumérées en une fois sous forme de tableaux numpy."""
import itertools, functools, numpy as np
@functools.lru_cache(maxsize=None)
def configs(n, R):
    # Pour chaque issue énumérée : nombre d'erreurs e et « décalage » d[i] ∈ {0, 1, 2} du signe réel par rapport au ticket.
    rows, errs = [], []
    for e in range(R):
        for wrong in itertools.combinations(range(n), e):
            for shifts in itertools.product((1, 2), repeat=e):
                d = [0] * n
                for i, s in zip(wrong, shifts): d[i] = s
                rows.append(d); errs.append(e)
    return np.array(rows, dtype=np.int64), np.array(errs, dtype=np.int64)
def ev(t, P, Q, S, masses, kappa=1.0, beta=1.0, floor=0.0, phi=0.0):
    """P, Q : tableaux (n, 3). Renvoie l'espérance par euro du ticket t."""
    n, R = P.shape[0], len(masses)
    D, E = configs(n, R)
    O = (np.asarray(t)[None, :] + D) % 3                      # signe réel de chaque match, pour chaque issue énumérée
    idx = np.arange(n)[None, :]
    prob = np.prod(P[idx, O], axis=1)
    a = Q[idx, O]                                             # part du public sur le signe réel
    dist = np.zeros((len(E), n + 1)); dist[:, 0] = 1.0       # loi de Poisson-binomiale du nombre de bons du public
    for i in range(n):
        x = a[:, i:i + 1]
        dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    lam = S * dist[np.arange(len(E)), n - E]
    lam = kappa * np.power(np.maximum(lam, 1e-300), beta) + floor + phi * S / 3 ** n   # plancher fixe, ou part φ des mises répartie sur les 3^n combinaisons
    share = np.where(lam < 1e-9, 1.0, -np.expm1(-lam) / np.maximum(lam, 1e-300))
    return float(np.sum(prob * np.asarray(masses)[E] * share))
