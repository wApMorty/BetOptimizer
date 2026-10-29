"""Collecte des grilles Loto Basket et Loto Rugby sur le site des points de vente FDJ (accord de Paul du 2026-10-28 :
lecture directe à faible débit, faits extraits seulement).

Les identifiants de grille (`internalDrawNumber`) sont communs au Basket 7 et au Rugby 7, et communs au Basket 15 et au
Rugby 15 ; l'adresse doit porter le bon jeu, sinon la page ne contient aucune grille. Pour chaque identifiant on essaie
donc les deux jeux. Une requête toutes les 5 s, une seule à la fois. On ne garde de chaque page que les faits utiles
(matchs, handicap, résultat, répartition s'il y en a une, rapports, garantie), jamais la page. Écriture dans un fichier
temporaire puis renommage ; reprise possible (les identifiants déjà vus, trouvés ou non, sont sautés).

Usage : python3 -I fdj_get.py DOSSIER [ID_MAX]
Journal horodaté sur stderr : identifiant en cours, n / N, durée écoulée, temps restant estimé.
"""
import json, os, re, sys, time, urllib.error, urllib.request

BASE = "https://www.pointdevente.parionssport.fdj.fr/grilles"
FAMILIES = {7: ["loto-basket/loto-basket-7", "loto-rugby/loto-rugby-7"],
            15: ["loto-basket/loto-basket-15", "loto-rugby/loto-rugby-15"]}
PAUSE = 5.0
T0 = time.time()


def log(msg):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), msg, file=sys.stderr, flush=True)


def fetch_state(path):
    """État JSON embarqué de la page (`serverApp-state`), ou None si la page n'en a pas."""
    req = urllib.request.Request(f"{BASE}/{path}", headers={"User-Agent": "edge-study/0.2"})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                s = r.read().decode("utf-8", errors="replace")
            break
        except (urllib.error.URLError, TimeoutError) as e:
            log(f"  erreur {e} sur {path}, nouvel essai dans {30 * (attempt + 1)} s")
            time.sleep(30 * (attempt + 1))
    else:
        raise RuntimeError(f"échec répété sur {path}")
    m = re.search(r'id="serverApp-state"[^>]*>(.*?)</script>', s, flags=re.S)
    if not m:
        return None
    st = m.group(1).replace("&q;", '"').replace("&a;", "&").replace("&s;", "'").replace("&l;", "<").replace("&g;", ">")
    return json.loads(st)


def facts(g):
    """Faits extraits d'une grille : rien d'autre n'est conservé."""
    return {
        "label": g["product"]["label"], "grid_number": g.get("gridNumber"), "id": g.get("internalDrawNumber"),
        "start_validation": g.get("startValidationDate"), "end_validation": g.get("endValidationDate"),
        "cancelled": g.get("drawCancelled"), "jackpot": (g.get("jackpot") or {}).get("amount"),
        "events": [{"teams": e.get("opponents"), "handicap": e.get("handicap"), "start": e.get("startDate"),
                    "result": e.get("result"),
                    "played": {o["label"]: [o.get("percentagePlayed"), o.get("numberPlayed")] for o in e.get("outcomes", [])}}
                   for e in g.get("events", [])],
        "winranks": [{"good": w.get("rankNumberMarket"), "winners": w.get("rankNumberGrid"), "gain_cents": w.get("rankAmount")}
                     for w in g.get("winranks") or []],
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }


def write(path, obj):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False)
    os.replace(tmp, path)


def main():
    root = sys.argv[1]
    id_max = int(sys.argv[2]) if len(sys.argv) > 2 else 460
    out = os.path.join(root, "grids")
    os.makedirs(out, exist_ok=True)
    # Les grilles 15 ont des identifiants bien plus petits (67 au 2026-10-28) : 1 à 90 suffit.
    todo = [(15, i) for i in range(90, 0, -1)] + [(7, i) for i in range(id_max, 0, -1)]
    todo = [t for t in todo if not os.path.exists(os.path.join(out, f"{t[0]}_{t[1]}.json"))]
    log(f"début : {len(todo)} identifiants à examiner, une requête toutes les {PAUSE:.0f} s")
    found = 0
    for n, (size, gid) in enumerate(todo, 1):
        rec = {"size": size, "id": gid, "grid": None}
        for fam in FAMILIES[size]:
            st = fetch_state(f"{fam}/{gid}")
            time.sleep(PAUSE)
            g = (st or {}).get("gridResolver")
            if g and g.get("product"):
                rec["grid"] = facts(g)
                found += 1
                break
        write(os.path.join(out, f"{size}_{gid}.json"), rec)
        elapsed = time.time() - T0
        eta = elapsed / n * (len(todo) - n)
        what = f"{rec['grid']['label']} n°{rec['grid']['grid_number']} {rec['grid']['end_validation'][:10]}" if rec["grid"] else "aucune grille"
        log(f"{n} / {len(todo)} · id {size}/{gid} : {what} · {found} trouvées · {elapsed/60:.1f} min écoulées, reste ≈ {eta/60:.0f} min")
    log("fini")


if __name__ == "__main__":
    main()
