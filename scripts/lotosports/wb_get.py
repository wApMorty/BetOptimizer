"""Collecte des copies Wayback des pages de grilles Loto Basket et Loto Rugby (site des points de vente FDJ, accord de Paul
du 2026-10-28). Le site FDJ ne sert plus que les deux derniers mois de grilles : l'historique vient d'archive.org.

Entrée : les listes CDX `cdx_<jeu>.txt` (horodatage, adresse, statut) du dossier. Une copie prise pendant la période de
validation porte la répartition du public ; une copie prise après les résultats porte les rapports. Les deux sont gardées.
Une requête à la fois, 4 s de pause (archive.org plafonne vers 15 pages par minute) ; sur 429 ou 503, pause de 60 s, puis
120 s, 240 s… Seuls les faits extraits sont écrits (`fdj_get.facts`), fichier temporaire puis renommage ; reprise possible.

Usage : python3 -I wb_get.py DOSSIER_WB
Journal horodaté sur stderr : copie en cours, n / N, durée écoulée, temps restant estimé.
"""
import glob, json, os, re, sys, time, urllib.error, urllib.request

# `python3 -I` retire le dossier du script du chemin d'import : on l'ajoute explicitement (nos scripts, pas les données).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fdj_get import facts, log, write  # noqa: E402

PAUSE = 4.0


def fetch(ts, url):
    backoff = 60
    for attempt in range(8):
        try:
            req = urllib.request.Request(f"https://web.archive.org/web/{ts}id_/{url}", headers={"User-Agent": "edge-study/0.2"})
            with urllib.request.urlopen(req, timeout=90) as r:
                return r.read().decode("utf-8", errors="replace")
        except urllib.error.HTTPError as e:
            if e.code in (429, 503):
                log(f"  HTTP {e.code} : pause de {backoff} s")
                time.sleep(backoff)
                backoff = min(backoff * 2, 1800)
            else:
                log(f"  HTTP {e.code} sur {url}")
                return None
        except (urllib.error.URLError, TimeoutError) as e:
            log(f"  erreur {e}, nouvel essai dans {15 * (attempt + 1)} s")
            time.sleep(15 * (attempt + 1))
    return None


def main():
    root = sys.argv[1]
    out = os.path.join(root, "json")
    os.makedirs(out, exist_ok=True)
    todo = []
    for f in sorted(glob.glob(os.path.join(root, "cdx_*.txt"))):
        for line in open(f):
            parts = line.split()
            m = re.search(r"/grilles/loto-(basket|rugby)/(loto-[a-z]+-\d+)/(\d+)", parts[1]) if len(parts) == 3 else None
            if m:
                name = f"{m.group(2)}_{m.group(3)}_{parts[0]}.json"
                if not os.path.exists(os.path.join(out, name)):
                    todo.append((name, parts[0], parts[1]))
    log(f"début : {len(todo)} copies à lire")
    t0, found = time.time(), 0
    for n, (name, ts, url) in enumerate(todo, 1):
        s = fetch(ts, url)
        time.sleep(PAUSE)
        rec = {"timestamp": ts, "url": url, "grid": None}
        if s is None:
            log(f"{n} / {len(todo)} · échec {name}")
            continue
        m = re.search(r'id="serverApp-state"[^>]*>(.*?)</script>', s, flags=re.S)
        if m:
            st = m.group(1).replace("&q;", '"').replace("&a;", "&").replace("&s;", "'").replace("&l;", "<").replace("&g;", ">")
            try:
                g = json.loads(st).get("gridResolver")
                if g and g.get("product"):
                    rec["grid"] = facts(g)
                    found += 1
            except (json.JSONDecodeError, KeyError, TypeError):
                pass
        write(os.path.join(out, name), rec)
        el = time.time() - t0
        what = f"{rec['grid']['label']} n°{rec['grid']['grid_number']}" if rec["grid"] else "sans grille"
        log(f"{n} / {len(todo)} · {name} : {what} · {found} lisibles · {el/60:.1f} min, reste ≈ {el/n*(len(todo)-n)/60:.0f} min")
    log("fini")


if __name__ == "__main__":
    main()
