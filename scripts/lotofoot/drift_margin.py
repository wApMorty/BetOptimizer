"""Regard exploratoire (question de Paul du 2026-10-25) : le ticket limite, juste au-dessus de τ, le reste-t-il jusqu'à la clôture ?

Même cadre que `drift_peek.py` (deux grilles closes, cotes et mises figées, aucune conclusion, protocole figé inchangé). Pour chaque
relevé : les 16 tickets de la recherche de production ; ceux au-dessus de τ (1,8, seuil à 60 € par mois ; 1,7 pour comparaison) ;
leur espérance S3 au dernier relevé (environ une heure avant la clôture) et au relevé suivant (une heure plus tard). Le second
rapport sert d'ordre de grandeur pour une heure de dérive de plus, celle qu'on ne voit pas avant la clôture.
"""
import sqlite3, sys, numpy as np
sys.path.insert(0, "."); import drift_peek as D

BUCKETS = [(0, 3), (3, 6), (6, 12), (12, 24)]


if __name__ == "__main__":
    c = sqlite3.connect(f"file:{D.DB}?mode=ro", uri=True)
    prm = D.PRM[8]
    for tau in (1.8, 1.7):
        stay = {b: [] for b in BUCKETS}          # (passe τ au relevé, passe encore τ au dernier relevé)
        worst = {b: [] for b in BUCKETS}         # espérance au dernier relevé du pire ticket retenu
        hour = []                                # rapport sur une heure des tickets retenus à moins de 0,4 au-dessus de τ, 6 dernières heures
        for game, number in (("lf8", 146), ("lf8", 147)):
            close, jackpot, P, S, readings, _, _ = D.grid(c, game, number)
            M = [0.7 * S * s for s in D.SHARES[game]]
            if jackpot:
                M[0] = max(M[0], jackpot)
            qs = [D.q_of(sh) for _, sh in readings]
            last = qs[-1]
            for i, ((at, _), Q) in enumerate(zip(readings, qs)):
                h = (close - at).total_seconds() / 3600
                b = next((b for b in BUCKETS if b[0] <= h < b[1]), None)
                T = D.B16.greedy(P, Q, S, M, prm, 16, 1)
                chosen = [(D.s3(t, P, Q, S, M, prm), t) for t in T]
                chosen = [(e, t) for e, t in chosen if e >= tau]
                if not chosen:
                    continue
                later = [D.s3(t, P, last, S, M, prm) for _, t in chosen]
                if b and i < len(readings) - 1:
                    stay[b] += [v >= tau for v in later]
                    worst[b].append(later[int(np.argmin([e for e, _ in chosen]))])
                if h <= 6 and i < len(readings) - 1:
                    nxt = qs[i + 1]
                    hour += [D.s3(t, P, nxt, S, M, prm) / e for e, t in chosen if e < tau + 0.4]
        print(f"\nτ = {tau}")
        for b in BUCKETS:
            if stay[b]:
                w = np.array(worst[b])
                print(f"  choisis {b[0]:>2}-{b[1]:<2} h avant : {np.mean(stay[b]):5.0%} passent encore τ au dernier relevé "
                      f"({len(stay[b])} tickets) ; pire ticket au dernier relevé : médiane {np.median(w):.2f}, min {w.min():.2f}")
        if hour:
            r = np.array(hour)
            print(f"  une heure de dérive, tickets limites (τ à τ + 0,4) des 6 dernières heures : rapport médian {np.median(r):.3f}, "
                  f"10e centile {np.percentile(r, 10):.3f}, min {r.min():.3f} ({len(r)} cas)")
