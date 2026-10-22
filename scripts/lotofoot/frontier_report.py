"""Analyse de la frontière (étape A) : par jeu, μ et k ; puis plan hebdomadaire de 10 € réparti sur les grilles disponibles."""
import json, sys, datetime as dt, collections, numpy as np
F = json.load(open(sys.argv[1] if len(sys.argv) > 1 else "../data/lf/frontier.json"))
MUS = ["0.0", "3.0", "10.0", "30.0"]
print("1. Par grille : espérance par euro et P(au moins un ticket payé), moyennes sur les grilles")
for game in (7, 8, 12, 15):
    sub = [r for r in F if r["game"] == game]
    print(f"LF{game} ({len(sub)} grilles)")
    for mu in MUS:
        cells = []
        for k in (1, 2, 5, 10):
            ev = np.mean([sum(r["mu"][mu]["ev"][:k]) / k for r in sub]); pa = np.mean([r["mu"][mu]["p_any"][k - 1] for r in sub])
            cells.append(f"k={k:>2}: {ev:4.2f} €/€, P {pa:5.1%}")
        print(f"   μ={mu:>4} | " + " | ".join(cells))
print("\n2. Plan de 10 €/semaine (tickets alloués un à un à la grille de meilleur gain marginal), semaines historiques")
weeks = collections.defaultdict(list)
for r in F: weeks[dt.date.fromisoformat(r["close"][:10]).isocalendar()[:2]].append(r)
print(f"   {len(weeks)} semaines avec au moins une grille appariée ; grilles par semaine : médiane {np.median([len(v) for v in weeks.values()]):.0f}")
for mu in MUS:
    for games in ((7,), (8,), (12,), (15,), (7, 8), (7, 8, 12, 15)):
        evw, paid, pweek = [], [], []
        for wk, gs in weeks.items():
            gs = [g for g in gs if g["game"] in games]
            if not gs: continue
            alloc = {(g["game"], g["gid"]): 0 for g in gs}; byid = {(g["game"], g["gid"]): g for g in gs}
            for _ in range(10):
                cand = [(byid[i]["mu"][mu]["ev"][alloc[i]], i) for i in alloc if alloc[i] < 10]
                if not cand: break
                alloc[max(cand)[1]] += 1
            ev = sum(sum(byid[i]["mu"][mu]["ev"][:k]) for i, k in alloc.items() if k)
            pa = [byid[i]["mu"][mu]["p_any"][k - 1] for i, k in alloc.items() if k]
            evw.append(ev); paid.append(sum(pa)); pweek.append(1 - np.prod([1 - x for x in pa]))
        if not evw: continue
        print(f"   μ={mu:>4} jeux {'/'.join(map(str, games)):9s}: {len(evw):3d} semaines, espérance {np.mean(evw):5.2f} € pour 10 € "
              f"(gain {52*(np.mean(evw)-10):+6.0f} €/an), grilles payées attendues {52*np.mean(paid):4.1f}/an, P(semaine payée) {np.mean(pweek):5.1%}")
