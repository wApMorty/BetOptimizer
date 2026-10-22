"""Étude de latence : les opérateurs français laissent-ils de la valeur contre Pinnacle près du coup d'envoi ?

PROTOCOLE FIGÉ LE 2026-10-21, AVANT TOUTE LECTURE DES DONNÉES OddsPapi (hors un match d'essai, Angers–Marseille).
- Données : historique OddsPapi, marché 1X2 (101), avant-match seulement ; une entrée = un changement de prix, valable
  jusqu'à l'entrée suivante ; une entrée `active = false` rend l'issue indisponible jusqu'à la suivante.
- Référence : Pinnacle sans marge, méthode puissance (comme la production), au même instant. Exige les trois issues
  actives chez Pinnacle et chez l'opérateur.
- Clôture : dernier prix actif de Pinnacle avant le coup d'envoi, sans marge (puissance).
- Horizons de décision : 72 h, 24 h, 6 h, 3 h, 90, 60, 45, 30, 15 et 5 min avant le coup d'envoi.
- Règle : pour chaque match, chaque opérateur et chaque horizon, l'issue de plus grand avantage o·p − 1 ; pari si ≥ 1 %
  (seuil de production).
- Critère principal : fenêtre des compositions = horizons 60, 45, 30 et 15 min ; Winamax et Unibet regroupés ; un pari au
  plus par match et par opérateur (le premier horizon qualifiant de la fenêtre). Succès si CLV moyen ≥ +1 % ET borne basse
  de l'intervalle à 95 % > 0 (bootstrap par blocs ligue × semaine ISO, 2 000 tirages, graine 7).
- Secondaires (descriptifs, sans décision) : CLV par horizon et par opérateur ; rendement réalisé (résultats de
  football-data) ; PMU, ZEbet ; Betfair comme seconde référence.
- Une seule configuration de décision : aucun seuil ni horizon ne sera ajusté après lecture.
"""
import json, glob, os, sys, random, collections, statistics as st, datetime as dt, math

HORIZONS_MIN = [72*60, 24*60, 6*60, 3*60, 90, 60, 45, 30, 15, 5]
WINDOW = [60, 45, 30, 15]
THRESHOLD = 0.01
OUT = {"101": "H", "102": "D", "103": "A"}

def ts(s): return dt.datetime.fromisoformat(s)
def power(odds):
    inv = [1/x for x in odds]; lo, hi = 0.5, 3.0
    for _ in range(80):
        k = (lo+hi)/2; s = sum(v**k for v in inv); lo, hi = (k, hi) if s > 1 else (lo, k)
    return [v**k for v in inv]

def price_at(series, t):
    """Dernier prix actif à l'instant t (None si aucun, ou si la dernière entrée est une suspension)."""
    last = None
    for c, p, a in series:
        if ts(c) <= t: last = (p, a, ts(c))
        else: break
    if last is None or not last[1]: return None
    return last

def triple(book, t):
    if not book or any(k not in book for k in OUT): return None
    r = {}
    for oid, o in OUT.items():
        x = price_at(book[oid], t)
        if x is None: return None
        r[o] = x
    return r

def load():
    data = collections.defaultdict(dict)
    for path in glob.glob("hist/*.json"):
        d = json.load(open(path))
        fid = d["fixture"]["fixtureId"]
        data[fid]["fixture"] = d["fixture"]
        for bk, m in d["odds"].items():
            if "101" in m:
                data[fid][bk] = {oid: sorted(v) for oid, v in m["101"].items()}
    return data

def bets(data, books):
    rows = []
    for fid, d in data.items():
        f = d["fixture"]; ko = ts(f["startTime"].replace(".000Z", "+00:00"))
        pin = d.get("pinnacle")
        close = triple(pin, ko - dt.timedelta(seconds=1))
        if not close: continue
        pc = dict(zip("HDA", power([close[o][0] for o in "HDA"])))
        week = f"{f['tournamentId']}-{ko.isocalendar()[0]}-{ko.isocalendar()[1]}"
        for bk in books:
            for h in HORIZONS_MIN:
                t = ko - dt.timedelta(minutes=h)
                p_now, b_now = triple(pin, t), triple(d.get(bk), t)
                if not p_now or not b_now: continue
                pn = dict(zip("HDA", power([p_now[o][0] for o in "HDA"])))
                o_best = max("HDA", key=lambda o: b_now[o][0]*pn[o])
                edge = b_now[o_best][0]*pn[o_best] - 1
                rows.append(dict(fid=fid, bk=bk, h=h, outcome=o_best, odds=b_now[o_best][0], edge=edge,
                                 clv=b_now[o_best][0]*pc[o_best] - 1, block=week, league=f["tournamentId"],
                                 book_age_min=(t - b_now[o_best][2]).total_seconds()/60,
                                 pin_age_min=(t - max(p_now[o][2] for o in "HDA")).total_seconds()/60,
                                 ko=f["startTime"], home=f["participant1Name"], away=f["participant2Name"]))
    return rows

def boot(rows, key="clv", n=2000, seed=7):
    if not rows: return (float("nan"),)*3
    by = collections.defaultdict(list)
    for r in rows: by[r["block"]].append(r[key])
    blocks = list(by.values()); rnd = random.Random(seed); means = []
    for _ in range(n):
        s = [x for _ in blocks for x in rnd.choice(blocks)]
        means.append(sum(s)/len(s))
    means.sort()
    return (st.mean(r[key] for r in rows), means[int(.025*n)], means[int(.975*n)])

if __name__ == "__main__":
    data = load()
    books = sys.argv[1].split(",") if len(sys.argv) > 1 else ["winamax.fr", "unibet.fr"]
    rows = bets(data, books)
    print("matchs chargés :", len(data), " lignes (match × opérateur × horizon) :", len(rows))
    print("\nPar horizon (toutes lignes, puis paris à ≥ 1 %) :")
    for h in HORIZONS_MIN:
        r = [x for x in rows if x["h"] == h]; b = [x for x in r if x["edge"] >= THRESHOLD]
        if not r: continue
        m, lo, hi = boot(b) if b else (float("nan"),)*3
        print(f"  {h:5d} min : n={len(r):5d} avantage moyen {100*st.mean(x['edge'] for x in r):+.2f} % | paris {len(b):4d} "
              f"CLV {100*m:+.2f} % [{100*lo:+.2f} ; {100*hi:+.2f}]")
    # critère principal
    first = {}
    for x in sorted((x for x in rows if x["h"] in WINDOW and x["edge"] >= THRESHOLD), key=lambda x: -x["h"]):
        first.setdefault((x["fid"], x["bk"]), x)
    sel = list(first.values())
    m, lo, hi = boot(sel)
    print(f"\nCRITÈRE PRINCIPAL (fenêtre 60–15 min, {','.join(books)}) : {len(sel)} paris, CLV {100*m:+.2f} % [{100*lo:+.2f} ; {100*hi:+.2f}]"
          f" → {'SUCCÈS' if m >= 0.01 and lo > 0 else 'ÉCHEC'}")
    for bk in books:
        s = [x for x in sel if x["bk"] == bk]
        if s:
            m, lo, hi = boot(s)
            print(f"   {bk:11s} {len(s):4d} paris, CLV {100*m:+.2f} % [{100*lo:+.2f} ; {100*hi:+.2f}], issues {dict(collections.Counter(x['outcome'] for x in s))}")
    json.dump(rows, open("latency_rows.json", "w"))
