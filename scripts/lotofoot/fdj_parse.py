"""Lecture des états JSON des grilles FDJ (copies Wayback) : matchs, vraie répartition, résultats, rapports, garantie."""
import gzip, json, glob, os, sys
RES = {"firstOpponentWinner": 0, "draw": 1, "secondOpponentWinner": 2}
def load(path):
    try: return _load(path)
    except (KeyError, TypeError, ValueError): return None, "structure inattendue"
def _load(path):
    d = json.load(gzip.open(path))
    try: st = json.loads(d["state"])
    except (json.JSONDecodeError, TypeError): return None, "état illisible"
    g = st.get("gridResolver")
    if not g or not g.get("events"): return None, "grille absente"
    if g.get("drawCancelled"): return None, "grille annulée"
    ev = sorted(g["events"], key=lambda e: e["order"])
    if any(e.get("result") not in RES for e in ev): return None, "résultat manquant"
    tot = [sum(o["numberPlayed"] for o in e["outcomes"]) for e in ev]
    if len(set(tot)) != 1 or tot[0] <= 0: return None, "totaux incohérents"
    S = tot[0]
    q = []
    for e in ev:
        by = {o["label"]: o["numberPlayed"] for o in e["outcomes"]}
        if set(by) != {"1", "N", "2"}: return None, "issues inattendues"
        q.append([by["1"] / S, by["N"] / S, by["2"] / S])
    wr = sorted(g.get("winranks") or [], key=lambda w: w["order"])
    if not wr: return None, "rapports absents"
    return {"game": int(d["game"]), "gid": int(d["gid"]), "num": g.get("gridNumber"), "close": g.get("endValidationDate"),
            "n": len(ev), "S": S, "q": q, "o": [RES[e["result"]] for e in ev],
            "teams": [e["opponents"] for e in ev], "start": [e["startDate"] for e in ev],
            "ranks": [{"good": w["rankNumberMarket"], "W": w["rankNumberGrid"], "gain": w["rankAmount"] / 100} for w in wr],
            "jackpot": g.get("jackpot"), "label": g.get("product", {}).get("label")}, None
if __name__ == "__main__":
    import collections
    why = collections.Counter(); ok = []
    for p in sorted(glob.glob(os.path.join(sys.argv[1], "*.json.gz"))):
        g, w = load(p)
        if g: ok.append(g)
        else: why[w] += 1
    print(len(ok), "grilles valides ; écartées :", dict(why))
    for g in ok[:5]:
        print(g["game"], g["num"], g["close"], g["n"], "S", g["S"], "rangs", [(r["good"], r["W"], r["gain"]) for r in g["ranks"]], "jackpot", g["jackpot"])
