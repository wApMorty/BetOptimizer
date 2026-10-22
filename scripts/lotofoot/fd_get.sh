#!/bin/bash
# Ligues supplémentaires de football-data.co.uk (même source que le projet), une requête toutes les 3 s.
cd "$1"
for s in 1516 1617 1718 1819 1920 2021 2122 2223 2324 2425 2526 2627; do
  for l in F2 E1 D2 I2 SP2 N1 P1 B1 T1 G1 E2 SC1 E0 SP1 D1 I1 F1 SC0; do
    f="${l}_${s}.csv"; [ -s "$f" ] && continue; [ -s "/home/paul/.local/share/edge/football-data/$f" ] && continue
    curl -sS -f -A "edge-study/0.2" -o "$f" "https://football-data.co.uk/mmz4281/$s/$l.csv" || rm -f "$f"
    sleep 3
  done
done
echo fini
