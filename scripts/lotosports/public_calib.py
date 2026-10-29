"""Le public du Loto Basket 7 et du Loto Rugby 7 se trompe-t-il ? Calibration de la répartition contre les résultats,
sur la période d'entraînement seulement (validation close avant le 2024-01-01) : la période 2024 et suivantes reste
intacte pour un test figé, comme au Loto Foot (PROTOCOLE_FDJ.md).

Pour chaque match : q = part des grilles du public sur chaque signe (copie Wayback ouverte la plus tardive, pourcentages
entiers), o = issue réelle. On compare :
  - la fréquence réelle du « N » à sa part moyenne dans le public ;
  - la fréquence de victoire du signe favori du public, par tranche de q ;
  - le facteur d'avantage moyen p/q d'un signe, avec p = fréquence observée dans sa tranche.
Si le public était calibré, fréquence observée ≈ q dans chaque tranche.

Usage : python3 -I public_calib.py DOSSIER [--all]   (--all inclut la période de test : à ne pas utiliser avant le gel)
"""
import glob, json, os, re, sys, time
from collections import defaultdict

RES = {"firstOpponentWinner": "1", "draw": "N", "secondOpponentWinner": "2"}
CUTOFF = "2024-01-01"


def log(msg):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), msg, file=sys.stderr, flush=True)


def load(root):
    """Par grille : répartition de la copie ouverte la plus tardive, résultats de n'importe quelle copie close."""
    split, result, meta = {}, {}, {}
    for f in glob.glob(os.path.join(root, "wb", "json", "*.json")) + glob.glob(os.path.join(root, "grids", "*.json")):
        rec = json.load(open(f, encoding="utf-8"))
        g = rec["grid"]
        if not g or g["cancelled"]:
            continue
        key = (g["label"], g["id"])
        ts = rec.get("timestamp") or re.sub(r"\D", "", g["captured_at"])
        meta[key] = g["end_validation"]
        if all(sum((v[0] or 0) for v in e["played"].values()) >= 95 for e in g["events"]):
            if key not in split or ts > split[key][0]:
                split[key] = (ts, [{k: (v[0] or 0) / 100 for k, v in e["played"].items()} for e in g["events"]],
                              [e["teams"] for e in g["events"]])
        if all(e["result"] in RES for e in g["events"]):
            result[key] = ([RES[e["result"]] for e in g["events"]], [e["teams"] for e in g["events"]])
    rows = []
    for key in split.keys() & result.keys():
        _, qs, teams_q = split[key]
        outs, teams_r = result[key]
        if teams_q != teams_r:
            continue  # composition changée entre la copie ouverte et la copie close
        for q, o in zip(qs, outs):
            rows.append({"label": key[0], "close": meta[key], "q": q, "o": o})
    return rows


def main():
    root = sys.argv[1]
    rows = load(root)
    if "--all" not in sys.argv:
        rows = [r for r in rows if r["close"] < CUTOFF]
    log(f"{len(rows)} matchs retenus")
    for label in ("Loto Basket 7", "Loto Rugby 7"):
        rs = [r for r in rows if r["label"] == label]
        if not rs:
            continue
        qn = sum(r["q"].get("N", 0) for r in rs) / len(rs)
        fn = sum(r["o"] == "N" for r in rs) / len(rs)
        print(f"\n{label} : {len(rs)} matchs. « N » : public {qn:.1%}, réalité {fn:.1%}")
        bins = defaultdict(lambda: [0, 0, 0.0])
        for r in rs:
            for s in ("1", "2"):
                q = r["q"].get(s, 0)
                b = min(int(q * 10), 9)
                bins[b][0] += 1
                bins[b][1] += r["o"] == s
                bins[b][2] += q
        print(f"  {'tranche q':<12}{'signes':>8}{'q moyen':>9}{'gagnés':>9}{'p/q':>7}")
        for b in sorted(bins):
            n, w, qs = bins[b]
            print(f"  {b/10:.1f}-{(b+1)/10:.1f}{'':<5}{n:>8}{qs/n:>9.1%}{w/n:>9.1%}{(w/n)/(qs/n):>7.2f}")


if __name__ == "__main__":
    main()
