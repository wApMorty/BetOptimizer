"""Regard exploratoire sur la dérive avant la clôture (question de Paul du 2026-10-25), sur les grilles closes déjà relevées.

Ne remplace pas l'étude figée (`PROTOCOLE_DERIVE.md`, 40 grilles) et n'en change aucune règle : deux grilles, aucune conclusion.
Pour chaque relevé horaire (un par `split_at`) : part du public sur le favori du marché ; meilleurs tickets de la recherche de
production sur la répartition du moment (cotes et mises figées, celles de la dernière recommandation) ; nombre de tickets ≥ 1,7 ;
rapport R = espérance S3 annoncée du meilleur ticket / sa valeur avec le dernier relevé ; gagnants du sans-faute prédits avec la
répartition du moment, à comparer au nombre publié (la seule trace de la répartition finale).
Base lue en lecture seule.
"""
import os, sys, json, sqlite3, datetime as dt, numpy as np
sys.path.insert(0, "."); import lf_fast, budget16 as B16
from frontier import PRM

DB = os.path.expanduser("~/.local/share/edge/edge.db")
SHARES = {"lf7": [0.45, 0.55], "lf8": [0.45, 0.55]}
OUT = {"H": 0, "D": 1, "A": 2}


def ts(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def s3(t, P, Q, S, M, prm):
    Pt = 0.9 * P + 0.1 * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    return lf_fast.ev_p(t, Pt, Q, S, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=1.4)[0]


def grid(c, game, number):
    close = ts(c.execute("SELECT closes_at FROM lotofoot_grid WHERE game=? AND number=?", (game, number)).fetchone()[0])
    jackpot = c.execute("SELECT jackpot FROM lotofoot_grid WHERE game=? AND number=?", (game, number)).fetchone()[0]
    inputs, stakes = c.execute("SELECT inputs, stakes_estimate FROM lotofoot_reco WHERE game=? AND number=? ORDER BY id DESC LIMIT 1",
                               (game, number)).fetchone()
    P = np.array(json.loads(inputs)["p"])
    rows = c.execute("""SELECT s.split_at, s.position, s.q_home, s.q_draw, s.q_away FROM lotofoot_split s
                        WHERE s.game=? AND s.number=? AND s.split_at IS NOT NULL ORDER BY s.split_at, s.position""", (game, number)).fetchall()
    splits = {}
    for at, pos, a, b, d in rows:
        splits.setdefault(at, {})[pos] = (a, b, d)
    n = P.shape[0]
    readings = [(ts(at), np.array([v[k] for k in range(1, n + 1)], float)) for at, v in splits.items() if len(v) == n]
    results = "".join(r[0] for r in c.execute("SELECT outcome FROM lotofoot_result WHERE game=? AND number=? ORDER BY position", (game, number)))
    ranks = c.execute("SELECT rank, winners, unit_gain FROM lotofoot_rank WHERE game=? AND number=? ORDER BY rank", (game, number)).fetchall()
    return close, jackpot, P, stakes, readings, results, ranks


def q_of(shares):
    s = np.maximum(shares, 0.5)
    return s / s.sum(axis=1, keepdims=True)


if __name__ == "__main__":
    c = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    prm = PRM[8]
    for game, number in (("lf8", 146), ("lf8", 147)):
        close, jackpot, P, S_est, readings, results, ranks = grid(c, game, number)
        n = P.shape[0]
        # Mises réelles déduites du 2e rang publié : masse = gain × gagnants = 0,70 · S · 0,55 (part conservatrice du §37).
        S_real = ranks[1][2] * ranks[1][1] / (0.7 * SHARES[game][1]) if len(ranks) > 1 else None
        M = [0.7 * S_est * s for s in SHARES[game]]
        if jackpot:
            M[0] = max(M[0], jackpot)
        fav = P.argmax(axis=1)
        last_q = q_of(readings[-1][1])
        o = [OUT[r] for r in results]
        print(f"\n{game.upper()} n°{number} : clôture {close:%d/%m %H:%M}Z, {len(readings)} relevés ; mises estimées {S_est:,.0f} €, "
              f"déduites des rapports {S_real:,.0f} € ; résultats {results} ; sans-faute publié : {ranks[0][1]} gagnants")
        print("   h avant   favori (pts)   meilleur S3   tickets ≥ 1,7   R du meilleur   gagnants sans-faute prédits")
        for at, shares in readings:
            Q = q_of(shares)
            h = (close - at).total_seconds() / 3600
            fav_share = 100 * np.mean([Q[i, fav[i]] for i in range(n)])
            T = B16.greedy(P, Q, S_est, M, prm, 16, 1)
            ev = sorted(((s3(t, P, Q, S_est, M, prm), t) for t in T), key=lambda x: -x[0])
            best, tbest = ev[0]
            r = best / s3(tbest, P, last_q, S_est, M, prm)
            lam = S_real * np.prod([Q[i, o[i]] for i in range(n)])
            mu = prm["kappa"] * lam ** prm["beta"] + prm["phi"] * S_real / 3 ** n
            print(f"   {h:6.1f}    {fav_share:6.1f}        {best:6.2f}        {sum(e >= 1.7 for e, _ in ev):3d}           {r:5.2f}         {mu:7.0f}")
