"""Avancement de la collecte des grilles FDJ archivées (fdj_wb_get.py) : jauge, débit, heure de fin estimée, détail par jeu.

Usage : python3 progress.py [DOSSIER] [--watch SECONDES]
DOSSIER contient targets.txt (la liste à collecter) et json/ (les grilles déjà collectées).
Lecture seule : le script ne touche à aucun fichier.
"""
import os, sys, time, glob, collections, datetime as dt, shutil

DEFAULT = "/tmp/claude-1000/-home-paul-Documents-Workspace-BetOptimizer/4458825f-9495-42a4-8e7a-11263abbcd17/scratchpad/data/lf/fdj"

def bar(frac, width):
    full = int(frac * width)
    return "█" * full + ("▌" if frac * width - full >= 0.5 else "") + "·" * (width - full - (1 if frac * width - full >= 0.5 else 0))

def snapshot(root):
    targets = [l.split() for l in open(os.path.join(root, "targets.txt")) if l.strip()]
    files = glob.glob(os.path.join(root, "json", "*.json.gz"))
    # L'heure de modification de chaque fichier donne le débit réel de la collecte.
    mtimes = sorted(os.path.getmtime(f) for f in files)
    done = {os.path.basename(f).split(".")[0] for f in files}
    per_game = collections.Counter(t[0] for t in targets)
    done_game = collections.Counter(k.split("_")[0][2:] for k in done)
    log = os.path.join(root, "get_log.txt")
    failures = sum(1 for l in open(log) if l.startswith("échec")) if os.path.exists(log) else 0
    finished = os.path.exists(log) and any(l.startswith("fini") for l in open(log))
    return len(targets), len(files), mtimes, per_game, done_game, failures, finished

def render(root):
    total, n, mtimes, per_game, done_game, failures, finished = snapshot(root)
    width = max(20, min(60, shutil.get_terminal_size((80, 20)).columns - 30))
    frac = n / total if total else 0.0
    lines = [f"Collecte des grilles FDJ archivées — {dt.datetime.now():%H:%M:%S}",
             f"  [{bar(frac, width)}] {frac:6.1%}   {n} / {total}"]
    # Débit sur les 30 dernières minutes (plus représentatif que depuis le début).
    recent = [t for t in mtimes if t >= time.time() - 1800]
    if len(recent) >= 2:
        rate = (len(recent) - 1) / (recent[-1] - recent[0]) * 60
        left = (total - n) / rate if rate > 0 else float("inf")
        eta = dt.datetime.now() + dt.timedelta(minutes=left)
        lines.append(f"  débit {rate:.1f} grilles/min · reste ≈ {left/60:.1f} h · fin estimée vers {eta:%H:%M}")
    if mtimes:
        idle = time.time() - mtimes[-1]
        lines.append(f"  dernière grille il y a {idle:.0f} s" + ("   ⚠ aucune grille depuis plus de 2 min" if idle > 120 and not finished else ""))
    for g in sorted(per_game, key=int):
        f = done_game[g] / per_game[g]
        lines.append(f"  LF{g:<3}[{bar(f, 20)}] {done_game[g]:4d} / {per_game[g]}")
    lines.append(f"  échecs : {failures}" + ("   — collecte terminée" if finished else ""))
    return "\n".join(lines)

if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if a != "--watch"]
    watch = None
    if "--watch" in sys.argv:
        i = sys.argv.index("--watch")
        watch = float(sys.argv[i + 1]) if i + 1 < len(sys.argv) else 30.0
        args = [a for a in args if a != sys.argv[i + 1]] if i + 1 < len(sys.argv) else args
    root = args[0] if args else DEFAULT
    if watch is None:
        print(render(root))
    else:
        try:
            while True:
                out = render(root)
                print("\033[2J\033[H" + out, flush=True)   # efface l'écran et remonte en haut
                if "collecte terminée" in out: break
                time.sleep(watch)
        except KeyboardInterrupt:
            pass
