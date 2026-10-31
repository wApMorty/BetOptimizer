"""Noyau du modèle d'optimisation du revenu net du Loto Foot (étude `net_revenue.py`).

Sur une grille de n rencontres, x_t ≥ 0 copies du ticket t. Le revenu net espéré est, pour une issue o et le rang r :
    V(x) = Σ_o P(o) Σ_r M_r · F(c_r(o), μ_r(o)) − Σ_t x_t,   F(c, μ) = c · E[1 / (c + W)],  W ~ Poisson(μ),
    c_0(o) = x_o,   c_1(o) = Σ_{t à une erreur de o} x_t.
F est concave en c : V est DR-sous-modulaire sur le treillis des entiers, et le glouton par valeur marginale a une garantie de
1 − 1/e (Soma & Yoshida 2018 ; Haugh & Singal 2021 s'appuient sur la même propriété pour les concours à répartition).
Formule de partage : Clair & Letscher 2007, `docs/sources.md` §37 et §63.
"""
import numpy as np

KMAX = 96
NODES, WEIGHTS = np.polynomial.legendre.leggauss(64)
NODES, WEIGHTS = (NODES + 1) / 2, WEIGHTS / 2


def share_table(mu, cmax=KMAX):
    """F[…, c] = c · E[1 / (c + W)], c = 0…cmax (quadrature de Gauss-Legendre, identique à `beyond_16.share_table`)."""
    start = np.maximum(0.0, 1 - 40 / np.maximum(mu, 1e-300))[..., None]
    x = start + (1 - start) * NODES
    weight = (1 - start) * WEIGHTS * np.exp(-mu[..., None] * (1 - x))
    out = np.zeros(mu.shape + (cmax + 1,))
    for c in range(1, cmax + 1):
        out[..., c] = c * weight.sum(axis=-1)
        weight = weight * x
    return out


def codes(n):
    return (np.arange(3 ** n)[:, None] // 3 ** np.arange(n)) % 3


def neighbours(n):
    """N[t, j] : les 2n tickets à exactement une erreur de t, en code base 3."""
    O = codes(n); powers = 3 ** np.arange(n); base = O @ powers
    cols = []
    for i in range(n):
        for d in (1, 2):
            cols.append(base + ((O[:, i] + d) % 3 - O[:, i]) * powers[i])
    return np.stack(cols, axis=1)


def poisson_binomial(a):
    """Loi du nombre de bons signes du public : a[m, i] = part du public sur le signe réel."""
    m, n = a.shape
    dist = np.zeros((m, n + 1)); dist[:, 0] = 1.0
    for i in range(n):
        x = a[:, i:i + 1]
        dist[:, 1:] = dist[:, 1:] * (1 - x) + dist[:, :-1] * x; dist[:, 0] *= (1 - x[:, 0])
    return dist


class Table:
    """Une grille dans un scénario : P(o) et F par issue et par rang, pour les 3^n issues. Le partage entre nos tickets est exact."""

    def __init__(self, P, Q, S, M, prm, low_mult=1.0, cmax=KMAX):
        n = P.shape[0]
        O = codes(n); idx = np.arange(n)[None, :]
        self.n, self.M, self.cmax = n, np.asarray(M, float), cmax
        self.prob = np.prod(P[idx, O], axis=1)
        dist = poisson_binomial(Q[idx, O])
        self.mu, self.dF = [], []
        for r in range(2):
            lam = S * dist[:, n - r]
            mu = prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** n
            mu = np.where(lam < 3, mu * low_mult, mu)
            self.mu.append(mu)
            self.dF.append(np.diff(share_table(mu, cmax), axis=1))   # dF[o, c] = F(c + 1) − F(c)
        self.N = neighbours(n)
        self.reset()

    def reset(self):
        size = 3 ** self.n
        self.c0, self.c1 = np.zeros(size, dtype=np.int64), np.zeros(size, dtype=np.int64)

    def marginals(self):
        """Valeur marginale de chaque ticket, partage avec nos tickets déjà choisis compris."""
        arange = np.arange(len(self.prob))
        gain0 = self.M[0] * self.prob * self.dF[0][arange, self.c0]
        rank1 = self.M[1] * self.prob * self.dF[1][arange, self.c1]        # issue o : nos tickets à une erreur de o
        return gain0 + rank1[self.N].sum(axis=1)

    def marginal_at(self, t):
        """Même valeur que `marginals()[t]`, pour un seul ticket."""
        near = self.N[t]
        return float(self.M[0] * self.prob[t] * self.dF[0][t, self.c0[t]]
                     + self.M[1] * np.sum(self.prob[near] * self.dF[1][near, self.c1[near]]))

    def add(self, t):
        self.c0[t] += 1
        self.c1[self.N[t]] += 1

    def walk(self, sequence):
        """Valeur cumulée V(k) des k premiers tickets d'une suite ordonnée."""
        self.reset()
        out = [0.0]
        for t in sequence:
            out.append(out[-1] + self.marginal_at(t))
            self.add(t)
        self.reset()
        return np.array(out)

    def greedy(self, count, copies=True):
        """Suite gloutonne et valeur marginale de chaque pas ; `copies=False` interdit de rejouer un ticket."""
        self.reset()
        sequence, values = [], []
        for _ in range(count):
            m = self.marginals()
            if not copies:
                m = np.where(self.c0 > 0, -1.0, m)
            t = int(np.argmax(m))
            sequence.append(t); values.append(float(m[t])); self.add(t)
        self.reset()
        return sequence, values


def code_of(ticket):
    return int(np.asarray(ticket) @ 3 ** np.arange(len(ticket)))


def s3_probabilities(P, Q, shrink=0.10):
    Pt = (1 - shrink) * P + shrink * Q
    return Pt / Pt.sum(axis=1, keepdims=True)
