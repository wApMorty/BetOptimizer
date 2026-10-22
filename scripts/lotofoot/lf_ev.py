"""Espérance exacte d'un ticket simple du Loto Foot (jeu à répartition).

Modèle (à valider) :
- issues des matchs indépendantes, de probabilités p (marché sans marge) ;
- grilles du public : S grilles d'un euro, signes cochés indépendamment match par match selon la répartition FDJ q ;
  le nombre de grilles du public à m erreurs pour une issue o suit une loi de Poisson de moyenne S · P_q(m erreurs | o),
  P_q étant une loi de Poisson-binomiale (convolution des a_i = q_i(o_i)) ;
- rang r (r = 1 : aucune erreur, r = 2 : une erreur…) : masse M_r partagée à parts égales entre les gagnants ;
  notre gain espéré au rang r, sachant o : M_r · E[1 / (1 + W)] = M_r · (1 − e^{−λ}) / λ, W ~ Poisson(λ).
Énumération exacte des issues à au plus R−1 erreurs pour le ticket (R rangs payés).
"""
import itertools, math
import numpy as np
def poisson_binomial(a):
    # P(nombre de réussites = k), a = probabilités de réussite indépendantes (convolution).
    d = np.zeros(len(a) + 1); d[0] = 1.0
    for x in a:
        d[1:] = d[1:] * (1 - x) + d[:-1] * x; d[0] *= (1 - x)
    return d
def share(lam):
    return 1.0 if lam < 1e-12 else (1 - math.exp(-lam)) / lam
def ev_ticket(t, p, q, S, masses):
    """t : signe choisi par match (0, 1, 2) ; p, q : listes de triplets ; S : grilles du public ; masses[r] : masse du rang r+1."""
    n, R = len(t), len(masses)
    ev, prank = 0.0, [0.0] * R
    for e in range(R):                       # nombre d'erreurs de notre ticket
        for wrong in itertools.combinations(range(n), e):
            alts = [[o for o in range(3) if o != t[i]] for i in wrong]
            for choice in itertools.product(*alts):
                o = list(t)
                for i, c in zip(wrong, choice): o[i] = c
                prob = math.prod(p[i][o[i]] for i in range(n))
                d = poisson_binomial([q[i][o[i]] for i in range(n)])     # d[k] : part du public à k bons
                lam = S * d[n - e]                                        # concurrents au même rang que nous
                ev += prob * masses[e] * share(lam); prank[e] += prob
    return ev, prank
def optimize(p, q, S, masses, starts, iters=6):
    best = None
    for t0 in starts:
        t = list(t0); cur = ev_ticket(t, p, q, S, masses)[0]
        for _ in range(iters):
            improved = False
            for i in range(len(t)):
                for o in range(3):
                    if o == t[i]: continue
                    t2 = list(t); t2[i] = o
                    v = ev_ticket(t2, p, q, S, masses)[0]
                    if v > cur + 1e-12: t, cur, improved = t2, v, True
            if not improved: break
        if best is None or cur > best[0]: best = (cur, t)
    return best
def ev_by_rank(t, p, q, S, masses, inflate=1.0):
    """Comme ev_ticket, mais détaille l'espérance par rang ; `inflate` multiplie le nombre de concurrents (sensibilité)."""
    n, R = len(t), len(masses); out = [0.0] * R
    for e in range(R):
        for wrong in itertools.combinations(range(n), e):
            alts = [[o for o in range(3) if o != t[i]] for i in wrong]
            for choice in itertools.product(*alts):
                o = list(t)
                for i, c in zip(wrong, choice): o[i] = c
                prob = math.prod(p[i][o[i]] for i in range(n))
                d = poisson_binomial([q[i][o[i]] for i in range(n)])
                out[e] += prob * masses[e] * share(inflate * S * d[n - e])
    return out
def ev_corrected(t, p, q, S, masses, kappa=1.0, beta=1.0, low_mult=1.0, low_cut=2.0, sigma=0.0, nquad=9):
    """Espérance avec λ_eff = κ λ^β (× low_mult si λ < low_cut) et un bruit multiplicatif log-normal par grille
    (écart-type sigma, quadrature de Gauss-Hermite) : le même facteur s'applique à tous les rangs d'une grille."""
    xs, ws = np.polynomial.hermite_e.hermegauss(nquad) if sigma > 0 else (np.array([0.0]), np.array([1.0]))
    ws = ws / ws.sum(); n, R = len(t), len(masses); tot = 0.0
    for e in range(R):
        for wrong in itertools.combinations(range(n), e):
            alts = [[o for o in range(3) if o != t[i]] for i in wrong]
            for choice in itertools.product(*alts):
                o = list(t)
                for i, c in zip(wrong, choice): o[i] = c
                prob = math.prod(p[i][o[i]] for i in range(n))
                lam = S * poisson_binomial([q[i][o[i]] for i in range(n)])[n - e]
                le = kappa * lam ** beta * (low_mult if lam < low_cut else 1.0)
                tot += prob * masses[e] * sum(w * share(le * math.exp(sigma * x - sigma ** 2 / 2)) for x, w in zip(xs, ws))
    return tot
