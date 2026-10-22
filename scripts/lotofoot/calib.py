"""Calibration du modèle du public sur les grilles passées (copies Wayback de secretsdujeu.com).

PROTOCOLE FIGÉ LE 2026-10-22, AVANT LECTURE DES RÉSULTATS :
- grilles retenues : tous leurs matchs appariés à football-data (deux équipes concordantes) avec un résultat FTR égal au
  signe publié et des cotes de clôture sans marge (Pinnacle, sinon Betfair, sinon moyenne) ;
- mises S : LF15, moyenne des masses des rangs 2 à 4 divisée par 0,14 (règle 40/20/20/20 de 70 %), grille écartée si ces
  masses diffèrent de plus de 10 % ; LF7, masse du rang 2 divisée par 0,385 (55 % de 70 %) ;
- public : q ∝ p^γ · e^{δ·nul}, γ = 1,670 et δ = 0,075 (ajustés sur les répartitions FDJ du 2026-10-22, gelés) ;
- prédiction : λ_r = S · P_q(n − r + 1 bons | issue réelle), loi de Poisson-binomiale ;
- mesure : régression de Poisson log E[W_r] = log κ + β log λ_r sur tous les rangs de toutes les grilles ;
  modèle juste si κ ≈ 1 et β ≈ 1. Intervalles par bootstrap des grilles (2 000 tirages, graine 20261022).
"""
import json, sys, math, datetime as dt, numpy as np
sys.path.insert(0, "."); import fd_match, lf_ev
G, D = 1.670, 0.075
def qmodel(p):
    w = [p[0] ** G, p[1] ** G * math.exp(D), p[2] ** G]; s = sum(w); return [x / s for x in w]
def build(grids, rows, verbose=False):
    out, skipped = [], []
    for g in grids:
        day = dt.date.fromisoformat(g["date"]); ms, why = [], None
        for m in g["matches"]:
            x, how = fd_match.match(rows, m["home"], m["away"], day, before=6, after=6)
            if x is None: why = f"non apparié : {m['home']} – {m['away']} ({how})"; break
            p, src = fd_match.fair(x["r"])
            if p is None: why = f"pas de cote : {x['home']}–{x['away']}"; break
            res = {"H": "1", "D": "N", "A": "2"}.get(x["ftr"])
            if m["res"] is None or res != m["res"]: why = f"résultat discordant : {m['home']}–{m['away']} {m['res']} / {x['ftr']}"; break
            ms.append({"p": p, "o": "1N2".index(res), "src": src, "how": how, "fd": f"{x['home']}–{x['away']} {x['day']}"})
        if why: skipped.append((g["game"], g["date"], g["num"], why)); continue
        rk = [r for r in g["ranks"] if r["winners"] is not None or r.get("raw")]
        out.append({**g, "ms": ms})
    return out, skipped
if __name__ == "__main__":
    grids = json.load(open(sys.argv[1])); rows = fd_match.load_fd()
    ok, sk = build(grids, rows)
    print(f"{len(ok)} grilles retenues, {len(sk)} écartées")
    for s in sk: print("  écartée", s)
    json.dump(ok, open(sys.argv[2], "w"), ensure_ascii=False, indent=1)
