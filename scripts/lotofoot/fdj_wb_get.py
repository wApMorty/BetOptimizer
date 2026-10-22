"""Collecte des copies Wayback des pages de grilles Loto Foot du site des points de vente FDJ (accord de Paul du 2026-10-22).
3 requêtes en parallèle, 2 s de pause par fil (archive.org plafonne de toute façon le débit à environ 15 pages/min par adresse, mesuré le 2026-10-22) ; si archive.org répond 429 ou 503, tous les
fils s'arrêtent 60 s (puis 120 s, 240 s…). On ne garde que l'état JSON embarqué (`serverApp-state`), compressé, écrit dans un
fichier temporaire puis renommé (un arrêt brutal ne laisse jamais de fichier tronqué). Reprise possible."""
import os, sys, re, time, gzip, json, threading, urllib.request, urllib.error
from concurrent.futures import ThreadPoolExecutor
D = sys.argv[1]; OUT = os.path.join(D, "json"); WORKERS, PAUSE = 3, 2.0
targets = [l.split() for l in open(os.path.join(D, "targets.txt"))]
todo = [t for t in targets if not os.path.exists(os.path.join(OUT, f"lf{t[0]}_{t[1]}.json.gz"))]
lock = threading.Lock(); state = {"resume_at": 0.0, "backoff": 60.0, "done": 0}
def wait_backoff():
    while True:
        with lock: until = state["resume_at"]
        if time.time() >= until: return
        time.sleep(min(5, until - time.time()))
def fetch(t):
    game, gid, ts, url = t
    path = os.path.join(OUT, f"lf{game}_{gid}.json.gz")
    for attempt in range(6):
        wait_backoff()
        try:
            req = urllib.request.Request(f"https://web.archive.org/web/{ts}id_/{url}", headers={"User-Agent": "edge-study/0.2"})
            with urllib.request.urlopen(req, timeout=90) as r: s = r.read().decode("utf-8", errors="replace")
            with lock: state["backoff"] = 60.0
            break
        except urllib.error.HTTPError as e:
            if e.code in (429, 503):
                with lock:
                    state["resume_at"] = max(state["resume_at"], time.time() + state["backoff"])
                    print(time.strftime("%H:%M:%S"), f"HTTP {e.code} : pause de {state['backoff']:.0f} s", flush=True)
                    state["backoff"] = min(state["backoff"] * 2, 1800)
            else:
                time.sleep(10)
            s = None
        except Exception:
            s = None; time.sleep(10 * (attempt + 1))
    if s is None:
        print("échec", game, gid, flush=True); return
    m = re.search(r'id="serverApp-state"[^>]*>(.*?)</script>', s, flags=re.S)
    st = (m.group(1) if m else "").replace("&q;", '"').replace("&a;", "&").replace("&s;", "'").replace("&l;", "<").replace("&g;", ">")
    tmp = path + ".tmp"
    with gzip.open(tmp, "wt") as f: json.dump({"game": game, "gid": gid, "ts": ts, "url": url, "state": st}, f)
    os.replace(tmp, path)
    with lock:
        state["done"] += 1
        if state["done"] % 100 == 0: print(time.strftime("%H:%M:%S"), state["done"], flush=True)
    time.sleep(PAUSE)
with ThreadPoolExecutor(WORKERS) as ex: list(ex.map(fetch, todo))
print("fini", flush=True)
