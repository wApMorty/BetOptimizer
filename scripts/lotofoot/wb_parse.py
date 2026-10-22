# Extraction d'une grille Loto Foot (résultats et rapports) d'une copie Wayback d'une page secretsdujeu.com.
import re, html, sys, json, glob, os
def num(s):
    s = s.replace("\xa0", "").replace(" ", "").replace("€", "").replace(",", ".")
    return float(s)
def parse(path):
    raw = open(path, "rb").read()
    try: s = raw.decode("utf-8")
    except UnicodeDecodeError: s = raw.decode("cp1252", errors="replace")
    h = re.search(r"<h2[^>]*>\s*Loto Foot (\d+)\s+N.\s*(\d+)\s*-\s*(\d\d)/(\d\d)/(\d{4})\s*</h2>", s)
    if not h: return None
    game, num_, d, m, y = h.groups()
    tbl = s[h.end(): s.find("</table>", h.end())]
    rows = []
    for tr in re.findall(r"<tr>(.*?)</tr>", tbl, flags=re.S):
        home = re.search(r"class=col_1[^>]*>(.*?)</td>", tr, flags=re.S)
        away = re.search(r"class=col_3[^>]*>(.*?)</td>", tr, flags=re.S)
        signs = re.findall(r'<p class="loto-foot-result([^"]*)">\s*([1N2])\s*</p>', tr)
        res = [sg for cls, sg in signs if "coche" in cls]
        if home and away:
            rows.append({"home": html.unescape(re.sub("<[^>]+>", "", home.group(1))).strip(),
                         "away": html.unescape(re.sub("<[^>]+>", "", away.group(1))).strip(),
                         "res": res[0] if len(res) == 1 else None, "nres": len(res)})
    g = s.find("Gains pour", h.end())
    t = html.unescape(re.sub(r"<[^>]+>", "|", s[g:g + 4000]))
    tk = [x.strip() for x in t.split("|") if x.strip()]
    ranks = []
    for i, x in enumerate(tk):
        mm = re.match(r"(\d+) bons pronostics", x)
        if mm and i + 2 < len(tk):
            try: ranks.append({"good": int(mm.group(1)), "winners": int(num(tk[i+1])), "gain": num(tk[i+2])})
            except ValueError:
                ranks.append({"good": int(mm.group(1)), "winners": None, "gain": None, "raw": tk[i+1:i+3]})
    return {"game": int(game), "num": int(num_), "date": f"{y}-{m}-{d}", "matches": rows, "ranks": ranks, "file": os.path.basename(path)}
if __name__ == "__main__":
    out = {}
    for p in sorted(glob.glob(sys.argv[1] + "/lf*_*.html")):
        g = parse(p)
        if g: out[(g["game"], g["date"], g["num"])] = g
    grids = list(out.values())
    json.dump(grids, open(sys.argv[2], "w"), ensure_ascii=False, indent=1)
    for g in grids:
        print(g["game"], g["date"], g["num"], len(g["matches"]), "".join(m["res"] or "?" for m in g["matches"]),
              [(r["good"], r["winners"], r["gain"]) for r in g["ranks"]])
