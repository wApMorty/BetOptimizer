"""Rapprochement des matchs d'une grille Loto Foot (noms français) avec football-data.co.uk (cotes de clôture).
Règle : on compare des noms normalisés (alias du projet + table d'étude), dans une fenêtre de dates ; le rapprochement
n'est accepté que si les DEUX équipes concordent dans le même match. Tout le reste est rapporté, jamais deviné."""
import csv, glob, os, re, unicodedata, datetime as dt, tomllib
from difflib import SequenceMatcher
ROOT = os.path.dirname(os.path.abspath(__file__))
FD_DIRS = [os.path.expanduser("~/.local/share/edge/football-data"), os.path.join(ROOT, "..", "data", "fd")]
ALIASES = tomllib.load(open("/home/paul/Documents/Workspace/BetOptimizer/config/aliases.toml", "rb"))
# Noms français des grilles FDJ → nom football-data (table d'étude, complétée à la lecture des non-appariés).
STUDY = {"fc barcelone": "barcelona", "milan ac": "milan", "ac milan": "milan", "juventus turin": "juventus", "as rome": "roma",
  "dep.corogne": "la coruna", "la corogne": "la coruna", "fc valence": "valencia", "valence": "valencia", "atleticomadrid": "ath madrid",
  "atletico madrid": "ath madrid", "atl. madrid": "ath madrid", "betis seville": "betis", "fc seville": "sevilla", "seville": "sevilla",
  "naples": "napoli", "inter milan": "inter", "lazio rome": "lazio", "bayern munich": "bayern munich", "gfc ajaccio": "ajaccio gfco",
  "st etienne": "st etienne", "saint-etienne": "st etienne", "paris sg": "paris sg", "psg": "paris sg", "man. city": "man city",
  "man. united": "man united", "manchester city": "man city", "manchester united": "man united", "nottingham f.": "nott'm forest",
  "athletic bilbao": "ath bilbao", "real sociedad": "sociedad", "celta vigo": "celta", "espanyol": "espanol", "mayence": "mainz",
  "cologne": "fc koln", "fribourg": "freiburg", "monchengladbach": "m'gladbach", "b.monchengladbach": "m'gladbach", "dortmund": "dortmund",
  "b. dortmund": "dortmund", "borussia dortmund": "dortmund", "leverkusen": "leverkusen", "francfort": "ein frankfurt", "e.francfort": "ein frankfurt",
  "hambourg": "hamburg", "brighton hove": "brighton", "tottenham": "tottenham", "west ham": "west ham", "newcastle": "newcastle",
  "wolverhampton": "wolves", "sheffield utd": "sheffield united", "leicester": "leicester", "parme": "parma", "genes": "genoa",
  "turin": "torino", "fiorentina": "fiorentina", "sampdoria": "sampdoria", "venise": "venezia", "rb leipzig": "rb leipzig", "lens": "lens",
  "le havre": "le havre", "paris fc": "paris fc", "brest": "brest", "marseille": "marseille", "lyon": "lyon", "lille": "lille"}
STUDY.update({"sporting braga": "sp braga", "chievo verone": "chievo", "fc porto": "porto", "benfica lisbon": "benfica", "benfica": "benfica",
  "exce.rotterdam": "excelsior", "spar.rotterdam": "sparta rotterdam", "atal.bergame": "atalanta", "hellas verone": "verona",
  "vitesse arnhem": "vitesse", "cadix": "cadiz", "cf valence": "valencia", "gerone": "girona", "evian thonon": "evian thonon gaillard",
  "cd tondela": "tondela", "sheffield wed.": "sheffield weds", "heart of midl.": "hearts", "rc genk": "genk", "kv beveren": "waasland-beveren",
  "athleticbilbao": "ath bilbao", "standard liege": "standard", "malines": "mechelen", "nacionalmadere": "nacional",
  "bourg-en-bresse": "bourg peronnas", "stoke city": "stoke", "spezia calcio": "spezia", "la hoya lorca": "lorca", "manchester utd": "man united",
  "manchestercity": "man city", "paris sg": "paris sg", "sportng lisbon": "sp lisbon", "sporting lisbonne": "sp lisbon", "sporting": "sp lisbon",
  "psv eindhoven": "psv eindhoven", "az alkmaar": "az alkmaar", "ajax amsterdam": "ajax", "feyenoord": "feyenoord", "fc twente": "twente",
  "fc groningue": "groningen", "fc utrecht": "utrecht", "anderlecht": "anderlecht", "club bruges": "club brugge", "la gantoise": "gent",
  "olympiakos": "olympiakos", "panathinaikos": "panathinaikos", "paok thessalo.": "paok", "galatasaray": "galatasaray", "fenerbahce": "fenerbahce",
  "besiktas": "besiktas", "celtic glasgow": "celtic", "rangers glasgow": "rangers", "aberdeen": "aberdeen", "real saragosse": "zaragoza",
  "saragosse": "zaragoza", "la corogne": "la coruna", "dep. la corogne": "la coruna", "real majorque": "mallorca", "majorque": "mallorca",
  "gijon": "sp gijon", "sporting gijon": "sp gijon", "rayo vallecano": "vallecano", "grenade": "granada", "malaga": "malaga", "levante": "levante",
  "hertha berlin": "hertha", "werder breme": "werder bremen", "schalke 04": "schalke 04", "hanovre": "hannover", "nuremberg": "nurnberg",
  "stuttgart": "stuttgart", "wolfsburg": "wolfsburg", "hoffenheim": "hoffenheim", "augsbourg": "augsburg", "union berlin": "union berlin",
  "bologne": "bologna", "udinese": "udinese", "cagliari": "cagliari", "sassuolo": "sassuolo", "empoli": "empoli", "palerme": "palermo",
  "atalanta bergame": "atalanta", "ac milan": "milan", "inter": "inter", "salernitana": "salernitana", "lecce": "lecce", "verone": "verona",
  "sheffield united": "sheffield united", "west bromwich": "west brom", "queens park r.": "qpr", "qpr": "qpr", "nottingham forest": "nott'm forest",
  "wolves": "wolves", "huddersfield": "huddersfield", "aston villa": "aston villa", "leeds": "leeds", "norwich city": "norwich", "swansea city": "swansea",
  "cardiff city": "cardiff", "stade rennais": "rennes", "ol. marseille": "marseille", "ol. lyonnais": "lyon", "as monaco": "monaco",
  "as saint-etienne": "st etienne", "st-etienne": "st etienne", "red star": "red star", "ac ajaccio": "ajaccio", "orleans": "orleans",
  "chateauroux": "chateauroux", "evian tg": "evian thonon gaillard"})
def core(n):
    words = [w for w in re.split(r"[\s.\-]+", n) if w and w not in {"fc", "cf", "as", "ac", "sc", "rc", "cd", "ud", "sd", "afc", "club", "city", "calcio", "real"}]
    return " ".join(words)
def norm(s):
    s = unicodedata.normalize("NFKD", s).encode("ascii", "ignore").decode().lower().strip()
    return re.sub(r"\s+", " ", s)
def canon_map():
    m = {}
    for lg, teams in ALIASES.items():
        for c, vs in teams.items():
            for v in [c] + vs: m.setdefault(norm(v), set()).add(norm(c))
    return m
CANON = canon_map()
def keys(name):
    n = norm(name); ks = {n}
    if n in STUDY: ks.add(norm(STUDY[n]))
    for k in list(ks): ks |= CANON.get(k, set())
    ks |= {core(k) for k in list(ks) if core(k)}
    return ks
def load_fd():
    rows = []
    for d in FD_DIRS:
        for p in glob.glob(os.path.join(d, "*.csv")):
            lg = os.path.basename(p).split("_")[0]
            with open(p, encoding="latin-1") as f:
                for r in csv.DictReader(f):
                    if not r.get("Date") or not r.get("HomeTeam"): continue
                    try: day = dt.datetime.strptime(r["Date"], "%d/%m/%Y" if len(r["Date"]) == 10 else "%d/%m/%y").date()
                    except ValueError: continue
                    rows.append({"lg": lg, "day": day, "home": r["HomeTeam"], "away": r["AwayTeam"], "ftr": r.get("FTR"),
                                 "hk": keys(r["HomeTeam"]), "ak": keys(r["AwayTeam"]), "r": r})
    # Ligues « new » de football-data : un fichier par pays, colonnes Home/Away/Res et cotes de clôture seulement.
    for p in glob.glob(os.path.join(ROOT, "..", "data", "fdnew", "*.csv")):
        lg = "new_" + os.path.basename(p)[:-4]
        with open(p, encoding="utf-8-sig", errors="replace") as f:
            for r in csv.DictReader(f):
                try: day = dt.datetime.strptime(r["Date"], "%d/%m/%Y").date()
                except (KeyError, ValueError): continue
                r = dict(r, HomeTeam=r["Home"], AwayTeam=r["Away"], FTR=r["Res"])
                rows.append({"lg": lg, "day": day, "home": r["Home"], "away": r["Away"], "ftr": r["Res"],
                             "hk": keys(r["Home"]), "ak": keys(r["Away"]), "r": r})
    return rows
GENERIC = {"real", "club", "city", "united", "sporting", "athletic", "atletico", "dynamo", "dinamo", "olympique", "racing",
           "sport", "stade", "rovers", "town", "county", "deportivo", "calcio", "union", "inter", "royal", "borussia", "eintracht"}
def words(keys):
    return {w for k in keys for w in re.split(r"[\s.\-'`]+", k) if len(w) >= 4 and w not in GENERIC}
def sim(a, b):
    return max(SequenceMatcher(None, x, y).ratio() for x in a for y in b)
_IDX = {}
def _index(rows):
    # Index des lignes par jour, construit une fois par liste de lignes (clé : identité de la liste).
    key = id(rows)
    if key not in _IDX:
        d = {}
        for x in rows: d.setdefault(x["day"], []).append(x)
        _IDX[key] = d
    return _IDX[key]
def match(rows, home, away, day, before=2, after=5):
    hk, ak = keys(home), keys(away)
    idx = _index(rows)
    cands = [x for k in range(-before, after + 1) for x in idx.get(day + dt.timedelta(days=k), [])]
    exact = [x for x in cands if hk & x["hk"] and ak & x["ak"]]
    if len(exact) == 1: return exact[0], "exact"
    # Mot significatif commun (≥ 4 lettres, hors mots génériques) pour CHACUNE des deux équipes, candidat unique.
    ht, at = words(hk), words(ak)
    byword = [x for x in cands if ht & words(x["hk"]) and at & words(x["ak"])]
    if len(byword) == 1: return byword[0], "mot commun"
    scored = sorted(((min(sim(hk, x["hk"]), sim(ak, x["ak"])), x) for x in cands), key=lambda t: -t[0])
    if scored and scored[0][0] >= 0.75 and (len(scored) == 1 or scored[1][0] < scored[0][0] - 0.1):
        return scored[0][1], f"approché {scored[0][0]:.2f}"
    return None, (f"meilleur {scored[0][0]:.2f} {scored[0][1]['home']}–{scored[0][1]['away']}" if scored else "aucun candidat")
def fair(r):
    """Probabilités sans marge (puissance) de la meilleure cote de clôture disponible : Pinnacle, Betfair, moyenne."""
    from market import power
    for cols in (("PSCH", "PSCD", "PSCA"), ("BFECH", "BFECD", "BFECA"), ("AvgCH", "AvgCD", "AvgCA"), ("PSH", "PSD", "PSA"), ("AvgH", "AvgD", "AvgA"), ("B365H", "B365D", "B365A")):
        try: o = [float(r[c]) for c in cols]
        except (KeyError, ValueError, TypeError): continue
        s = sum(1 / x for x in o)
        if 1.0 <= s <= 1.15: return power(o), cols[0]
    return None, None
