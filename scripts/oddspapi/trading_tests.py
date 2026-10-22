"""Tests inspirés du trading, sur l'historique OddsPapi.

ADDENDUM 2, FIGÉ LE 2026-10-21 AVANT LECTURE DES DONNÉES :
T1 — Momentum / retour à la moyenne de Pinnacle (Sung et al. 2019). Probabilité sans marge (puissance) du domicile et de
     l'extérieur. Pour t ∈ {24 h, 6 h, 3 h, 60 min} avant le coup d'envoi : x = p(t) − p(t − Δ), Δ = durée égale à t
     (bornée à 24 h) ; y = p(clôture) − p(t). Pente de y sur x (moindres carrés sans constante), intervalle par
     bootstrap par blocs ligue × semaine. Pente > 0 : momentum ; < 0 : retour à la moyenne. Descriptif ; une pente dont
     l'intervalle exclut 0 sera rapportée comme hypothèse à confirmer sur la saison suivante.
T2 — Demi-vie d'une cote française périmée. Pour chaque opérateur (Winamax, Unibet), chaque match et chaque issue 1X2 :
     épisodes où l'avantage o·p − 1 (p : Pinnacle sans marge au même instant) passe au-dessus de 1 % dans les 6 h avant le
     coup d'envoi ; durée jusqu'à ce que l'avantage repasse sous 0 (ou coup d'envoi, censure). Médiane et quantiles,
     épisodes censurés comptés à part.
"""
import json, collections, random, statistics as st, datetime as dt
from study import load, prices, power, MARKETS, ts

def t1(data):
    outs = MARKETS["101"]; labels = list(outs.values()); res = collections.defaultdict(list)
    for fid, d in data.items():
        f = d["fixture"]; ko = ts(f["startTime"].replace(".000Z", "+00:00")); pin = d["books"].get("pinnacle", {}).get("101")
        def prob(t):
            x = prices(pin, outs, t)
            return dict(zip(labels, power([x[o][0] for o in labels]))) if x else None
        pc = prob(ko - dt.timedelta(seconds=1))
        if not pc: continue
        block = f"{f['tournamentId']}-{ko.isocalendar()[0]}-{ko.isocalendar()[1]}"
        for tmin in (24*60, 6*60, 3*60, 60):
            p_t, p_0 = prob(ko - dt.timedelta(minutes=tmin)), prob(ko - dt.timedelta(minutes=tmin + min(tmin, 24*60)))
            if not p_t or not p_0: continue
            for o in ("H", "A"):
                res[tmin].append((block, p_t[o] - p_0[o], pc[o] - p_t[o]))
    print("T1 — pente du mouvement jusqu'à la clôture sur le mouvement passé (Pinnacle)")
    for tmin, v in res.items():
        slope = lambda s: sum(x*y for _, x, y in s) / sum(x*x for _, x, y in s)
        by = collections.defaultdict(list)
        for r in v: by[r[0]].append(r)
        blocks = list(by.values()); rnd = random.Random(7); bs = sorted(slope([r for _ in blocks for r in rnd.choice(blocks)]) for _ in range(2000))
        print(f"  t = {tmin:5d} min : n = {len(v):5d}, écart-type passé {st.pstdev(x for _, x, _ in v):.4f}, futur {st.pstdev(y for *_, y in v):.4f}, pente {slope(v):+.3f} [{bs[50]:+.3f} ; {bs[1950]:+.3f}]")

def t2(data, books=("winamax.fr", "unibet.fr")):
    outs = MARKETS["101"]; labels = list(outs.values())
    for bk in books:
        durs, cens = [], 0
        for fid, d in data.items():
            f = d["fixture"]; ko = ts(f["startTime"].replace(".000Z", "+00:00"))
            pin, bm = d["books"].get("pinnacle", {}).get("101"), d["books"].get(bk, {}).get("101")
            if not pin or not bm: continue
            start = ko - dt.timedelta(hours=6)
            # instants de changement de l'un ou l'autre, dans la fenêtre
            times = sorted({ts(c) for s in list(pin.values()) + list(bm.values()) for c, *_ in s if start <= ts(c) < ko} | {start})
            for o in labels:
                open_at = None
                for t in times:
                    pn_, bn = prices(pin, outs, t), prices(bm, outs, t)
                    if not pn_ or not bn:
                        continue
                    p = dict(zip(labels, power([pn_[x][0] for x in labels])))
                    e = bn[o][0] * p[o] - 1
                    if open_at is None and e >= 0.01: open_at = t
                    elif open_at is not None and e < 0:
                        durs.append((t - open_at).total_seconds() / 60); open_at = None
                if open_at is not None: cens += 1
        durs.sort()
        if durs:
            q = lambda a: round(durs[int(a * (len(durs) - 1))], 1)
            print(f"T2 — {bk} : {len(durs)} épisodes clos, {cens} censurés au coup d'envoi ; durée (min) médiane {q(.5)}, p25 {q(.25)}, p75 {q(.75)}, p90 {q(.9)}")
        else:
            print(f"T2 — {bk} : aucun épisode clos, {cens} censurés")

if __name__ == "__main__":
    data = load(); t1(data); t2(data)
