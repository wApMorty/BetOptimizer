import sqlite3, math, statistics as st, os
db = sqlite3.connect("file:" + os.path.expanduser("~/.local/share/edge/edge.db") + "?mode=ro", uri=True)

def power(odds):
    """Retrait de marge par la méthode « puissance » : p_i = q_i^k avec somme = 1 (q = 1/cote)."""
    q = [1/o for o in odds]
    lo, hi = 0.2, 5.0
    for _ in range(80):
        k = (lo+hi)/2
        s = sum(x**k for x in q)
        if s > 1: lo = k
        else: hi = k
    return [x**k for x in q]

rows = db.execute("""
SELECT g.id, g.season, g.match_date, g.ft_result,
  ps.home, ps.draw, ps.away, bf.home, bf.draw, bf.away, av.home, av.draw, av.away, m.moment
FROM game g
JOIN (SELECT 'close' moment UNION SELECT 'pre') m
JOIN game_odds ps ON ps.game_id=g.id AND ps.bookmaker='PS'  AND ps.moment=m.moment
JOIN game_odds bf ON bf.game_id=g.id AND bf.bookmaker='BFE' AND bf.moment=m.moment
LEFT JOIN game_odds av ON av.game_id=g.id AND av.bookmaker='Avg' AND av.moment=m.moment
""").fetchall()
idx = {'H':0,'D':1,'A':2}
for moment in ("pre","close"):
    sel = [r for r in rows if r[13]==moment]
    over_ps = [sum(1/o for o in r[4:7]) for r in sel]
    over_bf = [sum(1/o for o in r[7:10]) for r in sel]
    d_abs, ll_ps, ll_bf, ll_av, diff = [], [], [], [], []
    for r in sel:
        pp = power(r[4:7]); pb = power(r[7:10]); y = idx[r[3]]
        d_abs.append(sum(abs(a-b) for a,b in zip(pp,pb))/3)
        lp, lb = -math.log(pp[y]), -math.log(pb[y])
        ll_ps.append(lp); ll_bf.append(lb); diff.append(lb-lp)
        if r[10]:
            pa = power(r[10:13]); ll_av.append(-math.log(pa[y]))
    n=len(sel); m=st.mean(diff); se=st.stdev(diff)/math.sqrt(n)
    print(f"[{moment}] n={n}  surronde PS {st.mean(over_ps):.4f}  BFE {st.mean(over_bf):.4f} (min {min(over_bf):.3f}, max {max(over_bf):.3f})")
    print(f"   écart absolu moyen des probabilités sans marge : {st.mean(d_abs)*100:.2f} points  (médiane {st.median(d_abs)*100:.2f}, p95 {sorted(d_abs)[int(.95*n)]*100:.2f})")
    print(f"   log loss PS {st.mean(ll_ps):.4f}   BFE {st.mean(ll_bf):.4f}   Δ(BFE−PS) {m:+.5f} [{m-1.96*se:+.5f} ; {m+1.96*se:+.5f}]")
    if ll_av: print(f"   log loss Avg (même matchs) {st.mean(ll_av):.4f}")
    # par saison
    seasons = sorted(set(r[1] for r in sel))
    for s in seasons:
        sub=[r for r in sel if r[1]==s]
        print(f"      {s}: n={len(sub)}  dernière date {max(r[2] for r in sub)}")
