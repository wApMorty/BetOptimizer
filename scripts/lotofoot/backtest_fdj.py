"""Backtest Loto Foot sur les grilles FDJ archivées, protocole PROTOCOLE_FDJ.md (figé avant lecture)."""
import sys, json, math, datetime as dt, collections, numpy as np
sys.path.insert(0, "."); import lf_fast
import backtest_lf as B
SPLIT = "2024-01-01"
SHARES = {7: [0.45, 0.55], 8: [0.45, 0.55], 12: [0.4, 0.2, 0.2, 0.2], 15: [0.4, 0.2, 0.2, 0.2]}
def masses(g):
    m = [0.7 * g["S"] * s for s in SHARES[g["game"]]]
    jp = g.get("jackpot") or {}
    if jp.get("type") and jp.get("amount"): m[0] = max(m[0], float(jp["amount"]))
    return m
def run(grids, prm, label, use_p=True):
    B.K = 10
    B.ev_fast = lambda t, p, q, S, m: lf_fast.ev(t, np.asarray(p), np.asarray(q), S, m, kappa=prm["kappa"], beta=prm["beta"], phi=prm["phi"])
    res = []
    for g in grids:
        p = g["p"] if use_p else g["p_hat"]; q = g["q"]; n = g["n"]; M = masses(g); R = len(M)
        if len(g["ranks"]) < R: continue
        tk = B.choose(p, q, g["S"], M)
        Wpub = {n - r["good"]: r["W"] for r in g["ranks"]}          # rang (0 = sans faute) → gagnants publiés
        good = [sum(a == b for a, b in zip(t, g["o"])) for _, t in tk]
        ours = collections.Counter(n - x for x in good if n - x < R)
        won = sum(M[r] * c / (Wpub.get(r, 0) + c) for r, c in ours.items())
        res.append({"gid": g["gid"], "game": g["game"], "close": g["close"], "cost": len(tk), "won": won,
                    "ev": float(np.mean([e for e, _ in tk])), "ranks": dict(ours)})
    return res
def report(res, label):
    if not res: print(label, ": aucune grille"); return
    def roi(rs): c = sum(r["cost"] for r in rs); return sum(r["won"] for r in rs) / c - 1 if c else float("nan")
    weeks = collections.defaultdict(list)
    for r in res: weeks[dt.date.fromisoformat(r["close"][:10]).isocalendar()[:2]].append(r)
    keys = list(weeks); rng = np.random.default_rng(20261022); Bs = []
    for _ in range(10000):
        pick = [weeks[keys[i]] for i in rng.integers(0, len(keys), len(keys))]
        flat = [r for w in pick for r in w]; Bs.append(roi(flat))
    print(f"{label} : {len(res)} grilles, {sum(r['cost'] for r in res)} € misés, {sum(r['won'] for r in res):.0f} € gagnés, ROI {roi(res):+.1%} "
          f"[IC 95 % {np.percentile(Bs,2.5):+.1%} ; {np.percentile(Bs,97.5):+.1%}], EV annoncée {np.mean([r['ev'] for r in res]):.2f}, "
          f"grilles payées {sum(1 for r in res if r['won']>0)}")
    for gm in (7, 8, 12, 15):
        sub = [r for r in res if r["game"] == gm]
        if sub: print(f"   LF{gm:<2} {len(sub):4d} grilles, ROI {roi(sub):+7.1%}, EV annoncée {np.mean([r['ev'] for r in sub]):.2f}, payées {sum(1 for r in sub if r['won']>0)}, rangs {sum((collections.Counter(r['ranks']) for r in sub), collections.Counter())}")
def fit_gamma(grids):
    """q ∝ p^γ · e^{δ·nul} sur les matchs appariés de l'entraînement (divergence de Kullback-Leibler)."""
    from scipy.optimize import minimize
    pairs = [(p, q) for g in grids if g["close"] < SPLIT for p, q in zip(g["p"], g["q"]) if p is not None]
    def kl(x):
        tot = 0.0
        for p, q in pairs:
            w = np.array([p[0] ** x[0], p[1] ** x[0] * math.exp(x[1]), p[2] ** x[0]]); w /= w.sum()
            q_ = np.clip(np.array(q), 1e-6, 1); tot += float(np.sum(q_ * np.log(q_ / w)))
        return tot / len(pairs)
    r = minimize(kl, [1.5, 0.0], method="Nelder-Mead"); return r.x[0], r.x[1], len(pairs)
def debias(q, gam, dl):
    w = np.array([q[0], q[1] * math.exp(-dl), q[2]]) ** (1 / gam); return list(w / w.sum())
if __name__ == "__main__":
    games = {int(x) for x in sys.argv[2].split(",")}; tag = "_".join(sorted(sys.argv[2].split(","), key=int))
    grids = [g for g in json.load(open(sys.argv[1])) if g["game"] in games]; prm = json.load(open(f"../data/lf/fdj_params_{tag}.json"))
    print("paramètres gelés :", prm)
    test = [g for g in grids if g["close"] >= SPLIT]
    res = run(test, prm, "principal"); json.dump(res, open(f"../data/lf/bt_fdj_main_{tag}.json", "w"))
    report(res, "PRINCIPAL (toutes les grilles du test)")
    report([r for r in res if r["ev"] > 1.2], "SECONDAIRE (EV moyenne > 1,2)")
    noclose = {g["gid"] for g in test if not any(s.startswith("PSC") or s.startswith("AvgC") or s.startswith("B365C") for s in g["src"])}
    report([r for r in res if r["gid"] in noclose], "SENSIBILITÉ (aucune cote de clôture)")
    allg = [g for g in json.load(open(sys.argv[1].replace(".json", "_all.json"))) if g["game"] in games]
    gam, dl, npairs = fit_gamma(allg); print(f"\nfoule débiaisée : γ = {gam:.3f}, δ = {dl:.3f} sur {npairs} matchs d'entraînement")
    sec = []
    for g in allg:
        if g["close"] < SPLIT: continue
        g["p_hat"] = [p if p is not None else debias(q, gam, dl) for p, q in zip(g["p"], g["q"])]; sec.append(g)
    res2 = run(sec, prm, "secondaire", use_p=False); json.dump(res2, open(f"../data/lf/bt_fdj_crowd_{tag}.json", "w"))
    report(res2, "SECONDAIRE (toutes les grilles du test, foule débiaisée sur les matchs sans cote)")
