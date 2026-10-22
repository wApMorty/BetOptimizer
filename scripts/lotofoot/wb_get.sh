#!/bin/bash
# Télécharge les copies Wayback (une requête toutes les 4 s) des pages de résultats Loto Foot de secretsdujeu.com.
cd "$1"
for g in 15 7; do
  while read ts st; do
    f="lf${g}_${ts}.html"; [ -s "$f" ] && continue
    curl -sS -A "edge-study/0.2" -o "$f" "https://web.archive.org/web/${ts}id_/https://www.secretsdujeu.com/foot/loto-foot-resultat-$g" || rm -f "$f"
    sleep 4
  done < cdx$g.txt
done
echo fini
