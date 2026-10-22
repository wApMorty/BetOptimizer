"""Appariement des grilles FDJ (copies Wayback) avec les cotes de football-data. Écrit grids_fdj.json : p d'avant-match (ou
clôture pour les ligues « new », signalée), vraie répartition q, S, résultats, rapports. Le résultat FTR doit concorder."""
import json, sys, glob, os, datetime as dt, collections
sys.path.insert(0, "."); import fdj_parse, fd_match
from market import power
def pre(r, lg):
    cols = [("PSH", "PSD", "PSA"), ("AvgH", "AvgD", "AvgA"), ("B365H", "B365D", "B365A")]
    if lg.startswith("new_"): cols = [("PSCH", "PSCD", "PSCA"), ("AvgCH", "AvgCD", "AvgCA"), ("B365CH", "B365CD", "B365CA")]
    for c in cols:
        try: o = [float(r[x]) for x in c]
        except (KeyError, ValueError, TypeError): continue
        if 1.0 <= sum(1 / x for x in o) <= 1.15: return power(o), c[0]
    return None, None
if __name__ == "__main__":
    rows = fd_match.load_fd(); out = []; allg = []; why = collections.Counter(); unk = collections.Counter()
    for p in sorted(glob.glob(os.path.join(sys.argv[1], "*.json.gz"))):
        g, w = fdj_parse.load(p)
        if not g: why[w] += 1; continue
        ps, srcs, bad = [], [], None
        for (h, a), st, o in zip(g["teams"], g["start"], g["o"]):
            day = dt.date.fromisoformat(st[:10])
            x, how = fd_match.match(rows, h, a, day, before=1, after=1)
            pr, src = (pre(x["r"], x["lg"]) if x else (None, None))
            if x is not None and pr is not None and {"H": 0, "D": 1, "A": 2}.get(x["ftr"]) != o:
                bad = bad or "résultat discordant"; unk[f"DISCORDANT {h} – {a} / {x['home']}–{x['away']} {x['day']}"] += 1; pr = None
            if pr is None and not bad: bad = "non apparié" if x is None else "pas de cote"
            if x is None: unk[f"{h} – {a}"] += 1
            ps.append(pr); srcs.append(src)
        g["p"] = ps; g["src"] = srcs; allg.append(g)
        if bad: why[bad] += 1; continue
        out.append(g)
    json.dump(out, open(sys.argv[2], "w"))
    json.dump(allg, open(sys.argv[2].replace(".json", "_all.json"), "w"))   # pour l'analyse secondaire (p partiel)
    print(len(out), "grilles appariées ; écartées :", dict(why))
    print(collections.Counter(g["game"] for g in out))
    if len(sys.argv) > 3:
        for k, v in unk.most_common(int(sys.argv[3])): print(f"  {v:3d}  {k}")
