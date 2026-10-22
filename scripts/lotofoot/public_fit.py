"""Modèle de la répartition du public : q_o ∝ p_o^γ · exp(δ · [o = nul]), ajusté par minimum de divergence de
Kullback-Leibler entre la répartition FDJ observée et le modèle, sur les matchs des grilles en cours couverts par l'archive."""
import sys, math, numpy as np
sys.path.insert(0, "."); import pm_parse, market
from scipy.optimize import minimize
PAIRS = {"lf15": ["Paris Saint Germain|Lyon","RC Lens|Paris FC","AS Monaco|Lille","Rennes|Strasbourg","Auxerre|Le Mans FC","Angers|Lorient",
  "Marseille|Le Havre","Liverpool|Brighton and Hove Albion","Manchester United|Bournemouth","SC Freiburg|Bayern Munich","Lazio|Parma",
  "Inter Milan|Fiorentina","Celta Vigo|Real Betis","Atlético Madrid|Deportivo La Coruña","Barcelona|Real Madrid"],
  "lf8": ["Brest|Nice", None, None, None, None, None, "Torino|Monza", "Ipswich Town|Nottingham Forest"]}
def model(p, g, d):
    w = [p[0] ** g, p[1] ** g * math.exp(d), p[2] ** g]; s = sum(w); return [x / s for x in w]
def data():
    out = []
    for key, pairs in PAIRS.items():
        g = pm_parse.grid(f"../data/lf/pm_{key}.html")
        for pr, r in zip(pairs, g["matches"]):
            if not pr: continue
            h, a = pr.split("|"); m = market.latest(h, a)
            out.append((m["p"], [x / sum(r["q"]) for x in r["q"]], r["match"]))
    return out
if __name__ == "__main__":
    D = data()
    f = lambda x: sum(sum(qo * math.log(qo / mo) for qo, mo in zip(q, model(p, *x))) for p, q, _ in D)
    res = minimize(f, [1.5, 0.0], method="Nelder-Mead")
    g, d = res.x; print(f"{len(D)} matchs ; γ = {g:.3f}, δ = {d:.3f}, KL moyen {res.fun/len(D):.4f}")
    for p, q, name in D:
        print(f"{name:32s} p {[round(x,2) for x in p]}  q {[round(x,2) for x in q]}  modèle {[round(x,2) for x in model(p,g,d)]}")
