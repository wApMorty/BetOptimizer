"""Mesure sans modèle des grilles Loto Basket et Loto Rugby collectées par fdj_get.py : mises, garantie, retour du public.

Mises S d'une grille, déduites des rangs inférieurs (règlement Loto Sports v6, art. 7.6 et 10.1) :
  - grille 7 : le 2e rang reçoit 55 % de 70 % de S, donc S ≈ W₂ · g₂ / 0,385 ;
  - grille 15 : les rangs 2 à 4 reçoivent chacun 20 % de 70 % de S, donc S ≈ Σ W_r · g_r / 0,42.
Les gains des rangs inférieurs sont arrondis au dixième d'euro inférieur (art. 10.1.5) : S est sous-estimé d'au plus
0,10 · W / 0,385, ce qui est ajouté pour encadrer. Une grille dont un rang inférieur a été fusionné (gain unitaire minimum
ou absence de gagnant, art. 10.1.1 et 10.7) n'est pas utilisable et est écartée.

Retour du public : (sommes versées à tous les rangs) / S. Sans garantie il vaut au plus 0,70 ; au-dessus, la garantie
de 10 000 € du 1er rang (art. 10.1.7.5 et 10.1.7.6) y ajoute le fonds de report. C'est l'espérance d'une grille tirée
comme celles du public, avant toute sélection.

Usage : python3 -I payout.py DOSSIER
"""
import glob, json, os, statistics, sys, time
from collections import defaultdict

LOWER_SHARE = {7: 0.70 * 0.55, 15: 0.70 * 0.60}
TOP_SHARE = {7: 0.70 * 0.45, 15: 0.70 * 0.40}


def log(msg):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), msg, file=sys.stderr, flush=True)


def season(date):
    """Saison sportive de juillet à juin, notée par son année de début."""
    y, m = int(date[:4]), int(date[5:7])
    return y if m >= 7 else y - 1


def analyse(g, size):
    """Mises, versement et retour d'une grille ; None (et le motif) si elle n'est pas exploitable."""
    if g["cancelled"]:
        return None, "annulée"
    ranks = sorted(g["winranks"], key=lambda w: -w["good"])
    if len(ranks) != (2 if size == 7 else 4):
        return None, "rapports absents"
    if any(r["winners"] is None or r["gain_cents"] is None for r in ranks):
        return None, "rapports incomplets"
    lower = ranks[1:]
    if any(r["winners"] == 0 for r in lower):
        return None, "rang inférieur sans gagnant"
    paid_lower = sum(r["winners"] * r["gain_cents"] / 100 for r in lower)
    S = paid_lower / LOWER_SHARE[size]
    S_hi = (paid_lower + 0.10 * sum(r["winners"] for r in lower)) / LOWER_SHARE[size]
    top = ranks[0]["winners"] * ranks[0]["gain_cents"] / 100
    natural = TOP_SHARE[size] * S
    # Un rang 1 versé sous sa part naturelle signale une fusion de rangs (gain unitaire minimum) : grille écartée.
    if top and top < 0.9 * natural:
        return None, "rang 1 fusionné"
    return {"label": g["label"], "num": g["grid_number"], "date": g["end_validation"][:10], "season": season(g["end_validation"]),
            "jackpot": g["jackpot"] or 0, "S": S, "S_hi": S_hi, "W1": ranks[0]["winners"], "top": top, "natural": natural,
            "guarantee_used": top > natural * 1.02, "ret": (top + paid_lower) / S, "ret_lo": (top + paid_lower) / S_hi}, None


def best_copies(root):
    """Une grille par (jeu, identifiant) : relevé FDJ direct (grids/) et copies Wayback (wb/json/) réunis. On garde la
    copie qui a des rapports, la plus récente à défaut."""
    files = sorted(glob.glob(os.path.join(root, "grids", "*.json")) + glob.glob(os.path.join(root, "wb", "json", "*.json")))
    log(f"{len(files)} fichiers à lire")
    best = {}
    for f in files:
        g = json.load(open(f, encoding="utf-8"))["grid"]
        if not g:
            continue
        key = (g["label"], g["id"])
        score = (bool(g["winranks"]) and all(w["winners"] is not None for w in g["winranks"]), g["captured_at"])
        if key not in best or score > best[key][0]:
            best[key] = (score, g)
    return [g for _, g in best.values()]


def main():
    root = sys.argv[1]
    rows, why = [], defaultdict(int)
    for g in best_copies(root):
        r, w = analyse(g, 15 if g["label"].endswith("15") else 7)
        if r:
            rows.append(r)
        else:
            why[(g["label"], w)] += 1
    log(f"{len(rows)} grilles exploitables ; écartées : {dict(why)}")
    by = defaultdict(list)
    for r in rows:
        by[(r["label"], r["season"])].append(r)
        by[(r["label"], "toutes")].append(r)
    print(f"{'jeu':<15}{'saison':>8}{'grilles':>8}{'S médian':>10}{'pactole':>9}{'montant':>9}{'garantie':>10}{'sans 1er':>9}{'retour moyen':>14}{'médian':>8}{'> 100 %':>9}")
    for (label, s), rs in sorted(by.items(), key=lambda kv: (kv[0][0], str(kv[0][1]))):
        rets = [r["ret"] for r in rs]
        amounts = sorted({r["jackpot"] for r in rs if r["jackpot"]})
        shown = "-" if not amounts else f"{amounts[0]//1000}k" + (f"-{amounts[-1]//1000}k" if len(amounts) > 1 else "")
        print(f"{label:<15}{str(s):>8}{len(rs):>8}{statistics.median(r['S'] for r in rs):>10,.0f}"
              f"{sum(r['jackpot'] > 0 for r in rs):>9}{shown:>9}{sum(r['guarantee_used'] for r in rs):>10}{sum(r['W1'] == 0 for r in rs):>9}"
              f"{statistics.mean(rets):>13.0%} {statistics.median(rets):>7.0%}{sum(x > 1 for x in rets):>9}")
    json.dump(rows, open(os.path.join(root, "payout.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=0)
    log("tableau écrit ; détail par grille dans payout.json")


if __name__ == "__main__":
    main()
