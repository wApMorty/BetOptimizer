#!/usr/bin/env python3
# Jauge du téléchargement OddsPapi : une barre par passe, débit mesuré sur les 5 dernières minutes, fin estimée.
import json, os, glob, time, datetime as dt, subprocess
os.chdir(os.path.dirname(os.path.abspath(__file__)))
fx = json.load(open("fixtures_finished.json"))
recent = sum(f["startTime"] >= "2026-08-01" for f in fx)
since_april = sum(f["startTime"] >= "2026-04-07" for f in fx)  # l'historique d'OddsPapi commence début avril
passes = [("A", "Pinnacle, Winamax, Unibet", since_april), ("C", "Polymarket, Kalshi", recent),
          ("D", "bet365, bwin, PokerStars", recent), ("B", "PMU, ZEbet, Genybet", since_april)]
mt = {p: os.path.getmtime(p) for p in glob.glob("hist/*.json")}
now = time.time()
rate = sum(1 for t in mt.values() if now - t < 300) / 300  # fichiers par seconde
running = subprocess.run(["pgrep", "-f", "u download.py"], capture_output=True).returncode == 0
remaining = 0; total_all = 0
log = open("download.log").read()
print(f"Téléchargement OddsPapi — {dt.datetime.now():%H:%M:%S} — {'en cours' if running else 'ARRÊTÉ'}\n")
for tag, name, total in passes:
    done = sum(1 for p in mt if p.endswith(f"_{tag}.json"))
    status = ""
    if f"passe {tag} : 15 matchs" in log:  # la passe s'est arrêtée avant la fin de la liste
        total = done
        status = " abandon" if tag == "C" else " fini"
    elif done >= total:
        status = " fini"
    remaining += total - done; total_all += total
    w = 20; fill = round(w * done / total)
    print(f"  {tag}  {name:26s} [{'█' * fill}{'░' * (w - fill)}] {done:4d}/{total} {100 * done / max(total, 1):3.0f} %{status}")
print(f"\n  Total : {total_all - remaining}/{total_all} ({100 * (total_all - remaining) / total_all:.1f} %)")
if running and rate > 0:
    eta = dt.datetime.now() + dt.timedelta(seconds=remaining / rate)
    print(f"  Débit : {rate * 60:.1f} matchs/min — fin estimée vers {eta:%H:%M} (dans {remaining / rate / 3600:.1f} h)")
err = [l for l in log.splitlines() if ("HTTP" in l or "Error" in l) and "HTTP 404" not in l]
last = max(mt.values()) if mt else now
print(f"  Dernier match enregistré il y a {now - last:.0f} s")
print(f"  Sans historique (404) : {log.count('HTTP 404')} ; autres erreurs : {len(err)}")
print("  Historique depuis avril 2026 ; passe arrêtée après 15 matchs vides.")
