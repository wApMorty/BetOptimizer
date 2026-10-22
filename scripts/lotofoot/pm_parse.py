# Lecture des pages prono-match.fr (grille en cours d'un Loto Foot) : matchs et répartition FDJ.
import re, html, sys, json
def tokens(path):
    s = open(path, encoding="utf-8", errors="replace").read()
    s = re.sub(r"<script.*?</script>|<style.*?</style>", "", s, flags=re.S)
    t = html.unescape(re.sub(r"<[^>]+>", "|", s))
    return [x.strip() for x in t.split("|") if x.strip()]
def triple(tk, j):
    # « a · b · c » à partir de tk[j] ; renvoie (valeurs, indice suivant) ou (None, j)
    if j + 4 < len(tk) and tk[j].isdigit() and tk[j+1] == "·" and tk[j+2].isdigit() and tk[j+3] == "·" and tk[j+4].isdigit():
        return [int(tk[j]), int(tk[j+2]), int(tk[j+4])], j + 5
    return None, j
def grid(path):
    tk = tokens(path)
    head = next(x for x in tk if "grille ouverte" in x)
    k = next(i for i, x in enumerate(tk) if x == "Choix")
    rows, j = [], k + 1
    while j < len(tk) and tk[j].isdigit() and int(tk[j]) == len(rows) + 1:
        name = tk[j+1]; j += 2
        while triple(tk, j)[0] is None: j += 1   # note éventuelle (« le 2 est surjoué… »)
        q, j = triple(tk, j)
        m, j2 = triple(tk, j)
        if m is None: j += 1
        else: j = j2
        rows.append({"i": len(rows) + 1, "match": name, "q": q, "model": m})
        while j < len(tk) and not (tk[j].isdigit() and int(tk[j]) == len(rows) + 1 and j + 1 < len(tk) and not tk[j+1].isdigit()):
            if tk[j].startswith("Comment lire"): break
            j += 1
    return {"head": head, "matches": rows}
if __name__ == "__main__":
    for p in sys.argv[1:]:
        g = grid(p); print(g["head"])
        for r in g["matches"]: print(r)
def history(path):
    """Tableau des rapports des dernières grilles : [(n°, date, [(gain, gagnants) ou None par rang])]."""
    s = open(path, encoding="utf-8", errors="replace").read(); out = []
    for tr in re.findall(r"<tr><td>n°(\d+)</td><td>([^<]+)</td>(.*?)</tr>", s, flags=re.S):
        ranks = []
        for td in re.findall(r"<td>(.*?)</td>", tr[2], flags=re.S):
            if "aucun gagnant" in td: ranks.append(None); continue
            m = re.search(r"<b>([\d\s\xa0\u202f,]+) €</b><br><small>([\d\s\xa0\u202f]+) gagnants?</small>", td)
            if m: ranks.append((float(re.sub(r"[\s\xa0\u202f]", "", m.group(1)).replace(",", ".")), int(re.sub(r"[\s\xa0\u202f]", "", m.group(2)))))
        if ranks: out.append((int(tr[0]), tr[1], ranks))
    return out
