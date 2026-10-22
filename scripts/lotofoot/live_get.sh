#!/bin/bash
# Relevé horaire (étude ponctuelle) des grilles Loto Foot en cours et des derniers résultats sur prono-match.fr.
# Une page toutes les 5 s, un passage par heure ; fichiers horodatés, jamais écrasés.
cd "$1"
while true; do
  ts=$(date -u +%Y%m%dT%H%M%SZ)
  for g in 7 8 12 15; do
    curl -sS -f -A "edge-study/0.2" -o "lf${g}_${ts}.html" "https://prono-match.fr/loto-foot-$g/" || rm -f "lf${g}_${ts}.html"; sleep 5
  done
  curl -sS -f -A "edge-study/0.2" -o "res_${ts}.html" "https://prono-match.fr/resultats-loto-foot/" || rm -f "res_${ts}.html"
  sleep 3595
done
