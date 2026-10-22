"""Ticket simple d'espérance maximale pour chaque grille Loto Foot en cours (modèle d'indépendance du public)."""
import sys, numpy as np
sys.path.insert(0, "."); import pm_parse, market, lf_ev
NAMES = {
 7: ["TSG Hoffenheim|Lyon","Besiktas JK|Crystal Palace","Lillestrom|Real Sociedad","Celtic|Celta Vigo","Bournemouth|AC Milan","Ajax|Atalanta BC","AS Monaco|SC Freiburg"],
 8: ["Brest|Nice","USL Dunkerque|Metz","Pau FC|Grenoble","Nancy|Stade Lavallois","Clermont|Guingamp","Sochaux|Annecy FC","Torino|Monza","Ipswich Town|Nottingham Forest"],
 12: ["TSG Hoffenheim|Lyon","Besiktas JK|Crystal Palace","Lillestrom|Real Sociedad","Celtic|Celta Vigo","Bournemouth|AC Milan","Ajax|Atalanta BC","AS Monaco|SC Freiburg",
      "Lech Poznań|Sunderland","OFI Crete|Bayer Leverkusen","Olympiakos Piraeus|Sparta Prague","Juventus|Rennes","SK Sturm Graz|Marseille"],
 15: ["Paris Saint Germain|Lyon","RC Lens|Paris FC","AS Monaco|Lille","Rennes|Strasbourg","Auxerre|Le Mans FC","Angers|Lorient","Marseille|Le Havre",
      "Liverpool|Brighton and Hove Albion","Manchester United|Bournemouth","SC Freiburg|Bayern Munich","Lazio|Parma","Inter Milan|Fiorentina","Celta Vigo|Real Betis",
      "Atlético Madrid|Deportivo La Coruña","Barcelona|Real Madrid"]}
# Mises estimées (grilles récentes du même jour de la semaine) et masses : 70 % des mises.
SETUP = {7: (160e3, lambda S: [0.45 * 0.7 * S, 0.55 * 0.7 * S]),
         8: (520e3, lambda S: [max(0.315 * S, 150e3), 0.385 * S]),
         12: (550e3, lambda S: [max(0.28 * S, 200e3)] + [0.14 * S] * 3),
         15: (1.27e6, lambda S: [max(0.28 * S, 500e3)] + [0.14 * S] * 3)}
def load(game, path=None):
    g = pm_parse.grid(path or f"../data/lf/pm_lf{game}.html"); p, q, names = [], [], []
    for pair, r in zip(NAMES[game], g["matches"]):
        h, a = pair.split("|"); m = market.latest(h, a)
        if m is None: raise SystemExit(f"cote absente : {pair}")
        p.append(m["p"]); q.append([x / sum(r["q"]) for x in r["q"]]); names.append(r["match"])
    return g, p, q, names
if __name__ == "__main__":
    for game in (7, 8, 12, 15):
        g, p, q, names = load(game); S, mf = SETUP[game]; masses = mf(S); n = len(p)
        fav = [int(np.argmax(x)) for x in p]
        ratio = [int(np.argmax([p[i][o] / q[i][o] for o in range(3)])) for i in range(n)]
        best = lf_ev.optimize(p, q, S, masses, [fav, ratio])
        ev, pr = lf_ev.ev_ticket(best[1], p, q, S, masses)
        evf, prf = lf_ev.ev_ticket(fav, p, q, S, masses)
        byr = lf_ev.ev_by_rank(best[1], p, q, S, masses)
        print(f"LF{game} ({g['head'][:60]}) S≈{S/1e3:.0f} k€")
        print(f"   favoris {''.join('1N2'[x] for x in fav)} EV {evf:.3f}  |  optimal {''.join('1N2'[x] for x in best[1])} EV {ev:.3f}  par rang {[round(x,3) for x in byr]}  P(rangs) {[f'{x:.1e}' for x in pr]}")
