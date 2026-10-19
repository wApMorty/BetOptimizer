"""Diagnostic en lecture seule, sur la VALIDATION seule (2024-07-01 à 2025-06-30) : combien de prix mous B365/BW dépassent la
référence de 3 %, selon la référence (Pinnacle, Betfair Exchange, moyenne du marché), et quel CLV (contre la clôture de Pinnacle) ?"""
import sqlite3, os, statistics as st, math
db = sqlite3.connect("file:" + os.path.expanduser("~/.local/share/edge/edge.db") + "?mode=ro", uri=True)
def power(odds):
    q=[1/o for o in odds]; lo,hi=0.2,5.0
    for _ in range(80):
        k=(lo+hi)/2
        if sum(x**k for x in q)>1: lo=k
        else: hi=k
    return [x**k for x in q]
def load(period_sql):
    rows=db.execute(f"""SELECT g.id, g.season, g.ft_result, o.bookmaker, o.moment, o.home, o.draw, o.away FROM game g JOIN game_odds o ON o.game_id=g.id
      WHERE {period_sql} AND o.bookmaker IN ('PS','BFE','Avg','B365','BW','LB','BFD')""").fetchall()
    games={}
    for gid,season,res,b,m,h,d,a in rows:
        games.setdefault(gid,{"season":season,"res":res})[(b,m)]=(h,d,a)
    return games
idx={'H':0,'D':1,'A':2}
def run(games, book, ref, thr=0.03):
    n=0; clv=[]; won=0
    for gid,g in games.items():
        soft=g.get((book,"pre")); r=g.get((ref,"pre")); close=g.get(("PS","close"))
        if not soft or not r: continue
        if ref=="BFE" and sum(1/x for x in r)>1.10: continue
        p=power(r)
        best=max(range(3), key=lambda i: p[i]*soft[i]-1)
        edge=p[best]*soft[best]-1
        if edge<thr: continue
        n+=1
        if close:
            pc=power(close)[best]; clv.append(soft[best]*pc-1)
        won += (idx[g["res"]]==best)
    return n, clv, won
for label,sql in (("VALIDATION 2024/25 (1 980 matchs, Pinnacle présent partout)","g.match_date BETWEEN '2024-07-01' AND '2025-06-30'"),
                  ("ENTRAÎNEMENT 2019/20 à 2023/24 (pour la tendance)","g.match_date BETWEEN '2019-07-01' AND '2024-06-30'")):
    games=load(sql); print(label, len(games), "matchs")
    for book in ("B365","BW"):
        for ref in ("PS","BFE","Avg"):
            n,clv,won=run(games,book,ref)
            if clv:
                m=st.mean(clv); se=st.stdev(clv)/math.sqrt(len(clv)) if len(clv)>1 else float('nan')
                print(f"  {book:5s} contre {ref:4s}: {n:4d} paris ≥3 %  CLV moyen {m*100:+.2f} % ± {1.96*se*100:.2f}  (victoires {won}/{n})")
            else: print(f"  {book:5s} contre {ref:4s}: {n:4d} paris")
