"""Étude OddsPapi, version générale (1X2, plus/moins 2,5, BTTS).

Protocole principal : voir latency.py (figé le 2026-10-21, 1X2, Winamax + Unibet, fenêtre 60–15 min, seuil 1 %).
ADDENDUM FIGÉ LE 2026-10-21, AVANT LECTURE (seules la couverture et la fraîcheur ont été regardées) :
- Mêmes règles, appliquées aux marchés plus/moins 2,5 (1010 : plus 1010, moins 1011) et BTTS (104 : oui 104, non 105),
  avec Pinnacle sans marge (méthode puissance, deux issues) comme référence et clôture. Décision séparée par marché :
  succès si CLV moyen ≥ +1 % et borne basse > 0 dans la fenêtre 60–15 min, Winamax + Unibet regroupés.
- Trois marchés évalués : on rapporte le nombre de configurations (3 marchés × 1 fenêtre = 3 décisions) ; une décision
  « succès » isolée sur trois sera traitée comme à confirmer sur la saison suivante, pas comme un avantage établi.
- Descriptif : opérateurs des passes C, D et B (bet365.fr, bwin.fr, pokerstars.fr, pmu, zebet.fr, genybet.fr) ;
  Betfair Exchange et Polymarket comme références alternatives.
"""
import json, glob, sys, random, collections, statistics as st, datetime as dt

HORIZONS_MIN = [72*60, 24*60, 6*60, 3*60, 90, 60, 45, 30, 15, 5]
WINDOW = [60, 45, 30, 15]
THRESHOLD = 0.01
MARKETS = {"101": {"101": "H", "102": "D", "103": "A"}, "1010": {"1010": "O", "1011": "U"}, "104": {"104": "Y", "105": "N"}}

def ts(s): return dt.datetime.fromisoformat(s)
def power(odds):
    inv = [1/x for x in odds]; lo, hi = 0.5, 3.0
    for _ in range(80):
        k = (lo+hi)/2; s = sum(v**k for v in inv); lo, hi = (k, hi) if s > 1 else (lo, k)
    return [v**k for v in inv]
def price_at(series, t):
    last = None
    for c, p, a in series:
        if ts(c) <= t: last = (p, a, ts(c))
        else: break
    return None if last is None or not last[1] else last
def prices(book, outs, t):
    if not book or any(k not in book for k in outs): return None
    r = {}
    for oid, o in outs.items():
        x = price_at(book[oid], t)
        if x is None: return None
        r[o] = x
    return r

def load():
    data = collections.defaultdict(lambda: {"books": {}})
    for path in glob.glob("hist/*.json"):
        d = json.load(open(path)); fid = d["fixture"]["fixtureId"]
        data[fid]["fixture"] = d["fixture"]
        for bk, m in d["odds"].items():
            data[fid]["books"][bk] = {mid: {oid: sorted(v) for oid, v in mm.items()} for mid, mm in m.items()}
    return data

def rows_for(data, mid, books, ref="pinnacle"):
    outs = MARKETS[mid]; labels = list(outs.values()); rows = []
    for fid, d in data.items():
        f = d["fixture"]; ko = ts(f["startTime"].replace(".000Z", "+00:00"))
        pin = d["books"].get(ref, {}).get(mid)
        close = prices(pin, outs, ko - dt.timedelta(seconds=1))
        if not close: continue
        pc = dict(zip(labels, power([close[o][0] for o in labels])))
        y, w = ko.isocalendar()[0], ko.isocalendar()[1]
        for bk in books:
            bm = d["books"].get(bk, {}).get(mid)
            for h in HORIZONS_MIN:
                t = ko - dt.timedelta(minutes=h)
                pn_, bn = prices(pin, outs, t), prices(bm, outs, t)
                if not pn_ or not bn: continue
                pn = dict(zip(labels, power([pn_[o][0] for o in labels])))
                o = max(labels, key=lambda o: bn[o][0]*pn[o])
                rows.append(dict(fid=fid, bk=bk, h=h, outcome=o, odds=bn[o][0], edge=bn[o][0]*pn[o]-1,
                                 clv=bn[o][0]*pc[o]-1, block=f"{f['tournamentId']}-{y}-{w}", league=f["tournamentId"],
                                 over=sum(1/bn[x][0] for x in labels), p=pn[o], ko=f["startTime"],
                                 home=f["participant1Name"], away=f["participant2Name"]))
    return rows

def boot(vals_blocks, n=2000, seed=7):
    if not vals_blocks: return (float("nan"),)*3
    by = collections.defaultdict(list)
    for b, v in vals_blocks: by[b].append(v)
    blocks = list(by.values()); rnd = random.Random(seed); means = []
    for _ in range(n):
        s = [x for _ in blocks for x in rnd.choice(blocks)]; means.append(sum(s)/len(s))
    means.sort(); allv = [v for _, v in vals_blocks]
    return (sum(allv)/len(allv), means[int(.025*n)], means[int(.975*n)])

def report(rows, label):
    print(f"\n=== {label} : {len({r['fid'] for r in rows})} matchs, {len(rows)} lignes")
    for h in HORIZONS_MIN:
        r = [x for x in rows if x["h"] == h]; b = [x for x in r if x["edge"] >= THRESHOLD]
        if not r: continue
        m, lo, hi = boot([(x["block"], x["clv"]) for x in b])
        print(f"  {h:5d} min : n={len(r):5d} surronde {st.mean(x['over'] for x in r):.3f} avantage moyen {100*st.mean(x['edge'] for x in r):+6.2f} % | paris {len(b):4d} CLV {100*m:+6.2f} % [{100*lo:+.2f} ; {100*hi:+.2f}]")
    first = {}
    for x in sorted((x for x in rows if x["h"] in WINDOW and x["edge"] >= THRESHOLD), key=lambda x: -x["h"]):
        first.setdefault((x["fid"], x["bk"]), x)
    sel = list(first.values()); m, lo, hi = boot([(x["block"], x["clv"]) for x in sel])
    verdict = "SUCCÈS" if sel and m >= 0.01 and lo > 0 else "ÉCHEC"
    print(f"  FENÊTRE 60–15 min : {len(sel)} paris, CLV {100*m:+.2f} % [{100*lo:+.2f} ; {100*hi:+.2f}] → {verdict}")
    for bk in sorted({x['bk'] for x in sel}):
        s = [x for x in sel if x["bk"] == bk]; m, lo, hi = boot([(x["block"], x["clv"]) for x in s])
        print(f"     {bk:14s} {len(s):4d} paris, CLV {100*m:+.2f} % [{100*lo:+.2f} ; {100*hi:+.2f}], issues {dict(collections.Counter(x['outcome'] for x in s))}")
    return sel

if __name__ == "__main__":
    data = load()
    books = sys.argv[1].split(",") if len(sys.argv) > 1 else ["winamax.fr", "unibet.fr"]
    out = {}
    for mid, name in (("101", "1X2"), ("1010", "plus/moins 2,5"), ("104", "BTTS")):
        rows = rows_for(data, mid, books); out[mid] = rows
        report(rows, f"{name} — {','.join(books)}")
    json.dump(out, open("study_rows.json", "w"))
