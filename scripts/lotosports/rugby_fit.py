"""Loto Rugby 7 : ajustement sur l'entraînement (protocole PROTOCOLE_RUGBY.md, validé par Paul le 2026-10-29).

Lit les grilles collectées (`wb_get.py`, `fdj_get.py`), ne garde que celles closes avant le 2024-01-01, estime :
  - p : π_N (fréquence des « N ») et h (part des « 1 » parmi les matchs sans « N ») ;
  - concurrents : W ~ binomiale négative de moyenne κ λ^β + φ S / 3⁷ et de dispersion θ, maximum de vraisemblance sur les deux
    rangs (λ = S · P_q(r bons | issue réelle), loi de Poisson-binomiale, Clair & Letscher 2007) ;
  - analyse secondaire 1 : p(1 | pas de « N ») = σ(a + b · logit q̃₁), q̃₁ = q₁ / (q₁ + q₂) ;
puis écrit `params_rugby.json` et son empreinte SHA-256, et rapporte le ROI de l'entraînement (dans l'échantillon).
Les fonctions communes (chargement, espérance, gains) servent aussi à `rugby_test.py`.

Usage : python3 -I rugby_fit.py DOSSIER_LS   (venv des études : numpy, scipy)
Journal horodaté sur stderr.
"""
import glob, hashlib, itertools, json, math, os, re, statistics, sys, time
import numpy as np
from scipy.optimize import minimize
from scipy.special import gammaln

CUTOFF = "2024-01-01"
N = 7
RES = {"firstOpponentWinner": 0, "draw": 1, "secondOpponentWinner": 2}   # indices : 0 = « 1 », 1 = « N », 2 = « 2 »
SIGNS = ("1", "N", "2")
TOP, LOW = 0.70 * 0.45, 0.70 * 0.55                                       # règlement v6, art. 7.6 et 10.1
FLOOR_PCT = 0.25
STAKE = 1.0
T0 = time.time()

# Toutes les issues d'une grille de 7 matchs (3⁷ = 2 187), et pour chacune ses 14 voisines à une erreur.
OUT = np.array(list(itertools.product(range(3), repeat=N)))
POW = 3 ** np.arange(N - 1, -1, -1)
NEIGH = np.array([[int((o + (np.arange(N) == i) * (s - o[i])) @ POW) for i in range(N) for s in range(3) if s != o[i]] for o in OUT])


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')} +{time.time() - T0:5.0f} s] {msg}", file=sys.stderr, flush=True)


# ---------------------------------------------------------------- données

def load(root):
    """Grilles Loto Rugby 7 retenues (protocole, § Données) et S réel de toutes les grilles qui en ont un (pour Ŝ)."""
    qcopy, rescopy, rankcopy, meta = {}, {}, {}, {}
    files = glob.glob(os.path.join(root, "wb", "json", "loto-rugby-7_*.json")) + glob.glob(os.path.join(root, "grids", "7_*.json"))
    for f in files:
        rec = json.load(open(f, encoding="utf-8"))
        g = rec["grid"]
        if not g or g["label"] != "Loto Rugby 7" or g["cancelled"]:
            continue
        k = g["id"]
        ts = rec.get("timestamp") or re.sub(r"\D", "", g["captured_at"])[:14]
        meta[k] = (g["start_validation"], g["end_validation"], g["grid_number"])
        if g["events"] and all(sum((v[0] or 0) for v in e["played"].values()) >= 95 for e in g["events"]):
            if k not in qcopy or ts > qcopy[k][0]:
                qcopy[k] = (ts, g)
        if g["events"] and all(e["result"] in RES for e in g["events"]):
            rescopy[k] = g
        if len(g["winranks"]) == 2 and all(w["winners"] for w in g["winranks"]) and all(w["gain_cents"] for w in g["winranks"]):
            rankcopy[k] = g
    stakes = []                                   # (ouverture, clôture, S) de toutes les grilles à 2e rang exploitable
    for k, g in rankcopy.items():
        r2 = min(g["winranks"], key=lambda w: w["good"])
        stakes.append((meta[k][0], meta[k][1], r2["winners"] * r2["gain_cents"] / 100 / LOW))
    grids, dropped = [], 0
    for k in sorted(qcopy.keys() & rescopy.keys() & rankcopy.keys(), key=lambda k: meta[k][1]):
        gq, gr, gk = qcopy[k][1], rescopy[k], rankcopy[k]
        if len(gq["events"]) != N or [e["teams"] for e in gq["events"]] != [e["teams"] for e in gr["events"]]:
            dropped += 1
            continue
        q = np.array([[max((e["played"].get(s, [0])[0] or 0), FLOOR_PCT) for s in SIGNS] for e in gq["events"]], dtype=float)
        q /= q.sum(axis=1, keepdims=True)
        ranks = sorted(gk["winranks"], key=lambda w: -w["good"])
        S = ranks[1]["winners"] * ranks[1]["gain_cents"] / 100 / LOW
        grids.append({"id": k, "num": meta[k][2], "open": meta[k][0], "close": meta[k][1], "q": q,
                      "o": np.array([RES[e["result"]] for e in gr["events"]]), "W": [ranks[0]["winners"], ranks[1]["winners"]],
                      "S": S, "jackpot": gk["jackpot"] or 0})
    if dropped:
        log(f"{dropped} grilles écartées : composition différente entre la copie à répartition et la copie à résultats")
    return grids, sorted(stakes, key=lambda x: x[1])


def s_hat(g, stakes, train_median):
    """Ŝ ex ante : médiane de S des 8 dernières grilles closes avant l'ouverture de g (médiane de l'entraînement s'il y en a moins)."""
    prev = [s for _, c, s in stakes if c < g["open"]]
    return statistics.median(prev[-8:]) if len(prev) >= 8 else train_median


# ---------------------------------------------------------------- modèle

def p_flat(prm):
    """Probabilités du modèle principal, identiques pour les 7 matchs : (p₁, p_N, p₂)."""
    pn, h = prm["pi_n"], prm["h"]
    return np.array([(1 - pn) * h, pn, (1 - pn) * (1 - h)])


def p_logit(prm, q):
    """Analyse secondaire 1 : p(1 | pas de « N ») = σ(a + b · logit q̃₁), q̃₁ = q₁ / (q₁ + q₂) ; p(N) = π_N."""
    qt = q[:, 0] / (q[:, 0] + q[:, 2])
    s = 1 / (1 + np.exp(-(prm["a"] + prm["b"] * np.log(qt / (1 - qt)))))
    pn = prm["pi_n"]
    return np.stack([(1 - pn) * s, np.full(len(qt), pn), (1 - pn) * (1 - s)], axis=1)


def outcome_table(P, q, S):
    """Pour chaque issue o (3⁷) : P(o), λ₁(o) = S · Πq, λ₂(o) = S · P_q(6 bons | o) (Poisson-binomiale par convolution)."""
    P = np.broadcast_to(P, (N, 3))
    prob = np.prod(P[np.arange(N), OUT], axis=1)
    a = q[np.arange(N), OUT]                      # (3⁷, 7) : part du public sur le signe réalisé de chaque match
    d = np.zeros((len(OUT), N + 1))
    d[:, 0] = 1.0
    for i in range(N):
        x = a[:, i:i + 1]
        d[:, 1:] = d[:, 1:] * (1 - x) + d[:, :-1] * x
        d[:, 0] *= 1 - x[:, 0]
    return prob, S * d[:, N], S * d[:, N - 1]


def mu(lam, S, prm):
    """Moyenne des concurrents : κ λ^β + φ S / 3⁷ (modèle du Loto Foot, §37)."""
    return prm["kappa"] * np.power(np.maximum(lam, 1e-300), prm["beta"]) + prm["phi"] * S / 3 ** N


def inv_share(m, th):
    """E[1 / (1 + W)], W binomiale négative de moyenne m et de taille θ.
    E[1/(1+W)] = ∫₀¹ G(t) dt, G(t) = (1 + (m/θ)(1 − t))^(−θ) fonction génératrice ; avec a = m/θ :
    ∫₀¹ (1 + a s)^(−θ) ds = [1 − (1 + a)^(1−θ)] / (a (θ − 1)) pour θ ≠ 1, ln(1 + a) / a pour θ = 1."""
    a = np.asarray(m, dtype=float) / th
    small = a < 1e-9
    a_ = np.where(small, 1.0, a)
    if abs(th - 1) < 1e-9:
        v = np.log1p(a_) / a_
    else:
        v = (1 - np.power(1 + a_, 1 - th)) / (a_ * (th - 1))
    return np.where(small, 1.0, v)


def masses(S, jackpot):
    """M₁ = max(0,315 S, montant garanti), M₂ = 0,385 S (art. 7.6, 10.1.7.5)."""
    return max(TOP * S, jackpot), LOW * S


def ev_all(P, q, S, jackpot, prm):
    """Espérance (mise 1 €) de chacun des 3⁷ tickets :
    EV(c) = P(c) · M₁ · E[1/(1+W₁(c))] + Σ_{o à une erreur de c} P(o) · M₂ · E[1/(1+W₂(o))]."""
    prob, l1, l2 = outcome_table(P, q, S)
    M1, M2 = masses(S, jackpot)
    e1 = prob * M1 * inv_share(mu(l1, S, prm), prm["theta"])
    e2 = prob * M2 * inv_share(mu(l2, S, prm), prm["theta"])
    return e1 + e2[NEIGH].sum(axis=1)


def choose(ev, k):
    """Les k tickets d'espérance la plus haute (tous distincts : un ticket par issue)."""
    idx = np.argsort(-ev, kind="stable")[:k]
    return idx, OUT[idx]


# ---------------------------------------------------------------- gains

def payout(c1, c2, W1, W2, S, jackpot):
    """Gain total de nos tickets (vectorisé) : c₁ tickets à 7 bons, c₂ à 6 bons, W₁, W₂ autres gagnants, S mises du public.
    Nos mises s'ajoutent à S ; rang 1 arrondi à l'euro inférieur, rang 2 au dixième inférieur (art. 10.1.5) ; si le gain unitaire du
    rang 1 tombe sous celui du rang 2, les deux masses sont réunies et partagées entre tous leurs gagnants (art. 10.1.1)."""
    M1, M2 = masses(S, jackpot)
    c1, c2, W1, W2 = (np.asarray(x, dtype=float) for x in (c1, c2, W1, W2))
    n1, n2 = np.maximum(W1 + c1, 1), np.maximum(W2 + c2, 1)
    u1, u2 = np.floor(M1 / n1), np.floor(M2 / n2 * 10) / 10
    fused = (W1 + c1 > 0) & (u1 < u2)
    uf = np.floor((M1 + M2) / np.maximum(W1 + c1 + W2 + c2, 1) * 10) / 10
    return np.where(fused, (c1 + c2) * uf, c1 * u1 + c2 * u2)


def realized(g, tickets, n_tickets):
    """Gain réel des tickets sur la grille g, avec les gagnants publiés et S réel + nos mises."""
    good = (tickets == g["o"][None, :]).sum(axis=1)
    c1, c2 = int((good == N).sum()), int((good == N - 1).sum())
    return float(payout(c1, c2, g["W"][0], g["W"][1], g["S"] + STAKE * n_tickets, g["jackpot"])), c1, c2


# ---------------------------------------------------------------- ajustement

def observations(grids):
    obs = []
    for g in grids:
        a = g["q"][np.arange(N), g["o"]]
        d = np.zeros(N + 1)
        d[0] = 1.0
        for x in a:
            d[1:] = d[1:] * (1 - x) + d[:-1] * x
            d[0] *= 1 - x
        for r in range(2):
            obs.append({"id": g["id"], "rank": r + 1, "lam": g["S"] * d[N - r], "S": g["S"], "W": g["W"][r]})
    return obs


def nll(x, lam, S, W):
    # Binomiale négative, moyenne μ = κ λ^β + φ S / 3⁷, variance μ + μ²/θ (log-vraisemblance de fdj_calib.py, §37).
    k, b, fl, th = math.exp(x[0]), x[1], math.exp(x[2]), math.exp(x[3])
    m = k * lam ** b + fl * S / 3 ** N
    return float(-np.sum(gammaln(W + th) - gammaln(th) - gammaln(W + 1) + th * np.log(th / (th + m)) + W * np.log(m / (th + m))))


def fit_competitors(obs):
    lam = np.array([o["lam"] for o in obs])
    S = np.array([o["S"] for o in obs])
    W = np.array([o["W"] for o in obs], dtype=float)
    r = minimize(nll, [0.0, 1.0, math.log(0.05), math.log(5.0)], args=(lam, S, W), method="Nelder-Mead",
                 options={"maxiter": 20000, "xatol": 1e-8, "fatol": 1e-8})
    log(f"maximum de vraisemblance : convergé = {r.success}, −log L = {r.fun:.3f}, {r.nit} itérations")
    return {"kappa": math.exp(r.x[0]), "beta": float(r.x[1]), "phi": math.exp(r.x[2]), "theta": math.exp(r.x[3])}


def fit_logit(grids, pi_n):
    """a, b par maximum de vraisemblance logistique sur les matchs d'entraînement sans « N »."""
    x, y = [], []
    for g in grids:
        for i in range(N):
            if g["o"][i] != 1:
                qt = g["q"][i, 0] / (g["q"][i, 0] + g["q"][i, 2])
                x.append(math.log(qt / (1 - qt)))
                y.append(1.0 if g["o"][i] == 0 else 0.0)
    x, y = np.array(x), np.array(y)

    def f(v):
        z = v[0] + v[1] * x
        return float(np.sum(np.logaddexp(0, z) - y * z))
    r = minimize(f, [0.0, 1.0], method="Nelder-Mead", options={"xatol": 1e-9, "fatol": 1e-9})
    return {"a": float(r.x[0]), "b": float(r.x[1]), "pi_n": pi_n, "n": len(y)}


def calib_table(obs, prm, title):
    """Gagnants publiés contre gagnants prédits, par tranche de λ ; renvoie les rapports des tranches λ ≥ 10."""
    print(title)
    flags = []
    for lo, hi in ((0, 1), (1, 10), (10, 100), (100, 1e3), (1e3, 1e4), (1e4, 1e9)):
        sub = [o for o in obs if lo <= o["lam"] < hi]
        if not sub:
            continue
        w = sum(o["W"] for o in sub)
        m = float(sum(mu(np.array(o["lam"]), o["S"], prm) for o in sub))
        print(f"  λ ∈ [{lo:g} ; {hi:g}) : {len(sub):3d} obs, gagnants publiés {w:8d}, prédits {m:10.1f}, rapport {w / m:5.2f}")
        if lo >= 10:
            flags.append((lo, len(sub), w / m))
    return flags


def strategy(grids, stakes, train_median, prm, k, model="flat", use_real_s=False):
    """Tickets et gains de la stratégie sur une liste de grilles. Renvoie une ligne par grille."""
    rows = []
    for n, g in enumerate(grids, 1):
        S_choice = g["S"] if use_real_s else s_hat(g, stakes, train_median)
        P = p_flat(prm) if model == "flat" else p_logit(prm["logit"], g["q"])
        ev = ev_all(P, g["q"], S_choice, g["jackpot"], prm)
        idx, T = choose(ev, k)
        gain, c1, c2 = realized(g, T, k)
        rows.append({"id": g["id"], "num": g["num"], "close": g["close"], "S_hat": S_choice, "S": g["S"], "ev": float(ev[idx].sum()),
                     "cost": STAKE * k, "gain": gain, "c1": c1, "c2": c2, "tickets": T, "idx": idx, "P": P})
    return rows


def main():
    root = sys.argv[1]
    grids, stakes = load(root)
    train = [g for g in grids if g["close"] < CUTOFF]
    log(f"{len(grids)} grilles retenues, dont {len(train)} d'entraînement (closes avant le {CUTOFF}) ; seules celles-ci sont lues")
    signs = np.concatenate([g["o"] for g in train])
    pi_n = float(np.mean(signs == 1))
    h = float(np.sum(signs == 0) / np.sum(signs != 1))
    log(f"p : π_N = {pi_n:.4f} sur {len(signs)} matchs, h = {h:.4f}")
    obs = observations(train)
    log(f"ajustement des concurrents sur {len(obs)} observations (2 rangs × {len(train)} grilles)")
    prm = {"pi_n": pi_n, "h": h, **fit_competitors(obs)}
    prm["logit"] = fit_logit(train, pi_n)
    train_stakes = [s for _, c, s in stakes if c < CUTOFF]
    prm["train_median_S"] = statistics.median(train_stakes)
    prm["n_train_grids"] = len(train)
    prm["cutoff"] = CUTOFF
    path = os.path.join(root, "params_rugby.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(prm, f, ensure_ascii=False, indent=1, sort_keys=True)
    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
    print(f"paramètres gelés : {path}\nSHA-256 {digest}")
    print(json.dumps(prm, ensure_ascii=False, indent=1, sort_keys=True))
    calib_table(obs, prm, "CALIBRATION DES CONCURRENTS, ENTRAÎNEMENT (dans l'échantillon)")
    log("stratégie principale sur l'entraînement (dans l'échantillon)")
    train_stakes_rows = [x for x in stakes if x[1] < CUTOFF]       # Ŝ de l'entraînement : ne voit que l'entraînement
    rows = strategy(train, train_stakes_rows, prm["train_median_S"], prm, 10)
    cost, gain = sum(r["cost"] for r in rows), sum(r["gain"] for r in rows)
    print(f"ENTRAÎNEMENT, 10 tickets par grille (dans l'échantillon) : {len(rows)} grilles, {cost:.0f} € misés, {gain:.2f} € gagnés, "
          f"ROI {gain / cost - 1:+.1%} ; espérance annoncée {sum(r['ev'] for r in rows) / cost:.3f} € par euro ; "
          f"grilles payées {sum(r['gain'] > 0 for r in rows)} ; 1er rang {sum(r['c1'] for r in rows)}, 2e rang {sum(r['c2'] for r in rows)}")
    log("fini")


if __name__ == "__main__":
    main()
