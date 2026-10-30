"""Que coûte l'erreur d'estimation des mises S aux séances Loto Foot ? (2026-10-30)

Protocole, figé avant le calcul :
- S vrai : les mises publiées par la FDJ (`S` des grilles archivées). Production : `stakes_estimate`, médiane des 8 dernières grilles
  de la formule de masse du rang 2 / (0,55 × 0,70).
- walk-forward : une grille passée ne sert que si ses rapports sont connus à la décision. Décision = clôture − 8 h (âge minimal de
  production) ; rapports connus 3 h après le coup d'envoi de la dernière rencontre. Historique : masse du rang 2, jamais `S` publié,
  comme en production. Grilles évaluées : celles qui ont au moins 8 grilles passées de leur formule.
- estimateurs (8 configurations), du plus simple au moins simple :
    prod_8     médiane des 8 dernières, même formule (production) ;
    win_4      id., 4 dernières ;
    win_16     id., 16 dernières ;
    pooled_8   8 dernières LF7 et LF8 confondues ;
    day_8      8 dernières de la même formule, même jour de la semaine (Europe/Paris) ;
    day_jack_8 id., même jour et même présence d'un pactole ;
    reg        niveau (log de la médiane des 16 dernières) + régression de l'écart au niveau sur le jour, la présence du pactole
               et son log ; ridge α = 1, par formule, réajustée à chaque grille sur le passé ;
    reg_comp   id., plus la part de rencontres de Ligue 1, des quatre autres grands championnats et hors football-data (coupes,
               sélections, petits championnats).
  Repli : moins de 3 valeurs dans la case, on passe à l'estimateur plus simple (day_jack_8 → day_8 → prod_8) ; moins de 60 lignes
  pour la régression, day_jack_8.
- erreur : Ŝ / S − 1 par formule, sur toutes les grilles LF7 et LF8 archivées (`grids_fdj_all.json`).
- critère de choix, entraînement seul (grilles closes avant 2024) : erreur absolue moyenne de log Ŝ − log S ; à 0,01 près, le plus
  simple. Le test (2024 et après) n'est lu qu'ensuite (`--test`).
- effet : sur les 409 grilles appariées (`grids_fdj.json`, répartition finale, cotes d'avant-match), la recherche de production
  (gloutonne, μ = 10, 16 tickets distincts) est relancée pour chaque Ŝ. Masses et concurrents viennent de Ŝ, pactole en plancher du
  rang 1. Un ticket est joué si son espérance S3 **à Ŝ** atteint τ = 1,7 (abaque pour 100 € par mois) ; il est compté à son
  espérance S3 **à S vrai**. Référence « S connu » : même calcul avec Ŝ = S.
- mois simulés : ceux de `budget_tau.py` (rythmes 2025-2026, ordre aléatoire, budget mensuel sans report, 16 tickets par grille,
  4 000 années, arrivées communes à tous les estimateurs), budget de 100 € (60 et 200 € pour lecture).
- intervalle à 95 % des écarts : bootstrap de semaines ISO des grilles, 500 tirages de 200 années, appariés entre estimateurs.
- grilles du journal, pour lecture : tickets ≥ 1,7 à (Ŝ, q du journal), (S déduit, q du journal), (S déduit, dernière répartition).
Limites : espérances du modèle et non gains observés (aucun gain démontré, test figé du §37 échoué) ; répartition finale pour
l'historique ; archive lacunaire (les « 8 dernières » y sont plus anciennes qu'en production) ; τ de l'abaque gardé fixe, alors que
l'abaque a été calculé avec S connu ; S déduit des rapports environ 2 % au-dessus de S publié.

Usage, depuis scripts/lotofoot, en arrière-plan :
  ~/.local/share/edge/etudes/venv/bin/python stakes_error.py [--test] 2> ../data/lf/stakes_error.log > ../data/lf/stakes_error.txt &
Sans `--test`, aucune ligne du test n'est affichée. Calculs par grille en cache : ../data/lf/stakes_error_cache.pkl.
"""
import sys, json, os, pickle, sqlite3, time, datetime as dt, numpy as np
from collections import defaultdict
from multiprocessing import Pool
from statistics import median
from zoneinfo import ZoneInfo
sys.path.insert(0, "."); import lf_fast, budget16 as B16, fd_match
import backtest_fdj as BF
from frontier import PRM

K, YEARS, MIN_HISTORY, MIN_CELL, MIN_FIT = 16, 4000, 8, 3, 60
TAU, MAIN_BUDGET, BUDGETS = 1.7, 100, [60, 100, 200]
SHRINK, LOW_MULT, RANK2_SHARE = 0.1, 1.4, 0.55 * 0.70
DECISION_LEAD, REPORT_DELAY = dt.timedelta(hours=8), dt.timedelta(hours=3)
BOOTSTRAP, BOOT_YEARS = 500, 200
LAM = {7: 3.2 * 30.44 / 7, 8: 3.4 * 30.44 / 7}
PARIS = ZoneInfo("Europe/Paris")
NAMES = ["prod_8", "win_4", "win_16", "pooled_8", "day_8", "day_jack_8", "reg", "reg_comp"]
BIG4 = {"E0", "SP1", "D1", "I1"}
CACHE = "../data/lf/stakes_error_cache.pkl"
OUT = "../data/lf/stakes_error.json"
START = time.time()


def log(message):
    print(f"[{dt.datetime.now():%H:%M:%S} +{time.time() - START:6.0f} s] {message}", file=sys.stderr, flush=True)


def when(text):
    return dt.datetime.fromisoformat(text.replace("Z", "+00:00"))


def jackpot_of(g):
    jackpot = g.get("jackpot") or {}
    return float(jackpot.get("amount") or 0) if jackpot.get("type") else 0.0


def competitions(grids):
    """Part de rencontres de Ligue 1, des quatre autres grands championnats, et hors football-data, par grille (connu à l'avance)."""
    rows = fd_match.load_fd()
    log(f"football-data chargé, {len(rows)} lignes ; appariement des compétitions")
    out = {}
    for k, g in enumerate(grids, 1):
        leagues = []
        for (home, away), start in zip(g["teams"], g["start"]):
            found = fd_match.match(rows, home, away, dt.date.fromisoformat(start[:10]), before=1, after=1)
            leagues.append(found[0]["lg"] if found and found[0] else None)
        n = len(leagues)
        out[(g["game"], g["gid"])] = [sum(x == "F1" for x in leagues) / n, sum(x in BIG4 for x in leagues) / n,
                                      sum(x is None for x in leagues) / n]
        if k % 100 == 0 or k == len(grids):
            rate = (time.time() - START) / k
            log(f"compétitions {k} / {len(grids)}, reste environ {rate * (len(grids) - k):.0f} s")
    return out


def series(grids, comp):
    """Une ligne par grille, triée par clôture : tout ce qu'un estimateur a le droit de lire, plus la vérité `S`."""
    rows = []
    for g in grids:
        second = [r for r in g["ranks"] if r["good"] == g["n"] - 1 and r["W"] > 0 and r["gain"]]
        close = when(g["close"])
        rows.append({"game": g["game"], "gid": g["gid"], "num": g["num"], "close": close, "train": g["close"] < "2024", "S": float(g["S"]),
                     "decision": close - DECISION_LEAD, "known": max(when(s) for s in g["start"]) + REPORT_DELAY,
                     "deduced": second[0]["W"] * second[0]["gain"] / RANK2_SHARE if second else None,
                     "day": close.astimezone(PARIS).weekday(), "jackpot": jackpot_of(g), "comp": comp[(g["game"], g["gid"])]})
    return sorted(rows, key=lambda r: r["close"])


def tail_median(values, window):
    return float(median(values[-window:]))


def features(row, with_comp):
    x = [1.0] + [float(row["day"] == d) for d in range(1, 7)] + [float(row["jackpot"] > 0), np.log(row["jackpot"] / 1e5) if row["jackpot"] else 0.0]
    return x + row["comp"] if with_comp else x


def ridge(row, past, with_comp):
    """Écart prédit entre log S et le niveau, ajusté sur `past` : β = (XᵀX + αI)⁻¹ Xᵀy, α = 1."""
    X = np.array([features(r, with_comp) for r in past])
    y = np.array([np.log(r["deduced"]) - r["level"] for r in past])
    beta = np.linalg.solve(X.T @ X + np.eye(X.shape[1]), X.T @ y)
    return float(np.array(features(row, with_comp)) @ beta)


def estimate(rows):
    """Remplit `row["est"]` en walk-forward. `queue` est triée par heure de connaissance : rien n'y entre avant la décision de la grille."""
    pending = sorted((r for r in rows if r["deduced"]), key=lambda r: r["known"])
    known, cursor = [], 0
    for k, row in enumerate(rows, 1):
        while cursor < len(pending) and pending[cursor]["known"] <= row["decision"]:
            known.append(pending[cursor]); cursor += 1
        same = [r for r in known if r["game"] == row["game"]]
        row["level"] = np.log(tail_median([r["deduced"] for r in same], 16)) if same else None
        if len(same) < MIN_HISTORY:
            row["est"] = None
            continue
        masses = [r["deduced"] for r in same]
        est = {"prod_8": tail_median(masses, 8), "win_4": tail_median(masses, 4), "win_16": tail_median(masses, 16),
               "pooled_8": tail_median([r["deduced"] for r in known], 8)}
        by_day = [r["deduced"] for r in same if r["day"] == row["day"]]
        est["day_8"] = tail_median(by_day, 8) if len(by_day) >= MIN_CELL else est["prod_8"]
        by_cell = [r["deduced"] for r in same if r["day"] == row["day"] and (r["jackpot"] > 0) == (row["jackpot"] > 0)]
        est["day_jack_8"] = tail_median(by_cell, 8) if len(by_cell) >= MIN_CELL else est["day_8"]
        fit = [r for r in same if r["level"] is not None]
        for name, with_comp in (("reg", False), ("reg_comp", True)):
            est[name] = float(np.exp(row["level"] + ridge(row, fit, with_comp))) if len(fit) >= MIN_FIT else est["day_jack_8"]
        row["est"] = est
        if k % 200 == 0 or k == len(rows):
            log(f"estimateurs {k} / {len(rows)}")


def error_line(label, rows, name, truth="S"):
    ratio = np.array([r["est"][name] / r[truth] for r in rows])
    rel = np.abs(ratio - 1)
    print(f"{label:30s} {len(rows):5d}   {np.median(ratio) - 1:+6.1%}   {np.median(rel):6.1%}   {rel.mean():6.1%}   {np.percentile(rel, 90):6.1%}"
          f"   {np.mean(rel > 0.2):6.1%}   {np.abs(np.log(ratio)).mean():.3f}")


def error_table(rows, title):
    print(f"\n{title}")
    print("estimateur                         n    biais   médiane  moyenne     p90     > 20 %   |log|")
    for name in NAMES:
        error_line(name, rows, name)


def detail_table(rows, title):
    print(f"\n{title}")
    print("production, par groupe              n    biais   médiane  moyenne     p90     > 20 %   |log|")
    for game in (7, 8):
        sub = [r for r in rows if r["game"] == game]
        error_line(f"LF{game}", sub, "prod_8")
        error_line(f"LF{game} sans pactole", [r for r in sub if not r["jackpot"]], "prod_8")
        error_line(f"LF{game} avec pactole", [r for r in sub if r["jackpot"]], "prod_8")


def evaluate_grid(g):
    """Pour chaque Ŝ : tickets recherchés à Ŝ, classés par espérance S3 à Ŝ, avec leur espérance S3 à S vrai."""
    P, Q = np.array(g["p"]), np.array(g["q"]); prm = PRM[g["game"]]; S = g["S"]; M = BF.masses(g)
    Pt = (1 - SHRINK) * P + SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    ev = lambda t, stakes, masses: lf_fast.ev_p(t, Pt, Q, stakes, masses, prm["kappa"], prm["beta"], prm["phi"], low_mult=LOW_MULT)[0]
    out = {}
    for name, stakes in g["estimates"].items():
        masses = BF.masses({**g, "S": stakes})
        T = B16.greedy(P, Q, stakes, masses, prm, K, 1)
        announced = np.array([ev(t, stakes, masses) for t in T])
        real = np.array([ev(t, S, M) for t in T])
        order = np.argsort(-announced, kind="stable")
        out[name] = (announced[order], real[order])
    return (g["game"], g["gid"]), out


def effects(grids):
    if os.path.exists(CACHE):
        log(f"tickets lus dans {CACHE}")
        with open(CACHE, "rb") as f:
            return pickle.load(f)
    done, out = 0, {}
    with Pool(8) as pool:
        for key, value in pool.imap_unordered(evaluate_grid, grids, chunksize=2):
            out[key] = value; done += 1
            if done % 10 == 0 or done == len(grids):
                rate = (time.time() - START) / done
                log(f"grilles {done} / {len(grids)}, reste environ {rate * (len(grids) - done):.0f} s")
    with open(CACHE, "wb") as f:
        pickle.dump(out, f)
    return out


def tables_for(results, keys, name):
    """Par grille : tickets au-dessus de τ à Ŝ, et cumuls de (espérance − 1) à S vrai et telle qu'annoncée."""
    announced = np.array([results[k][name][0] for k in keys]); real = np.array([results[k][name][1] for k in keys])
    zero = np.zeros((len(keys), 1))
    return {"demand": (announced >= TAU).sum(axis=1), "real": np.hstack([zero, np.cumsum(real - 1, axis=1)]),
            "announced": np.hstack([zero, np.cumsum(announced - 1, axis=1)])}


def arrivals(idx, rng, years):
    months = years * 12
    counts = {k: rng.poisson(LAM[k], months) for k in (7, 8)}
    width = int((counts[7] + counts[8]).max())
    G = np.full((months, width), -1, dtype=np.int64)
    for m in range(months):
        row = np.concatenate([rng.choice(idx[7], counts[7][m]), rng.choice(idx[8], counts[8][m])])
        rng.shuffle(row); G[m, :len(row)] = row
    return G


def simulate(table, G, budget, years):
    valid = G >= 0; gi = np.where(valid, G, 0)
    demand = np.where(valid, table["demand"][gi], 0)
    before = np.cumsum(demand, axis=1) - demand
    k = np.clip(budget - before, 0, demand)
    return {"real": float((table["real"][gi, k] * valid).sum() / years), "announced": float((table["announced"][gi, k] * valid).sum() / years),
            "spend": float(k.sum(axis=1).mean()), "capped": float(np.mean(k.sum(axis=1) == budget))}


def budget_table(tables, keys, games, mask, seed, title):
    idx = {k: np.flatnonzero(mask & (games == k)) for k in (7, 8)}
    G = arrivals(idx, np.random.default_rng(seed), YEARS)
    log(f"{title} : {len(G)} mois simulés")
    out = {}
    for budget in BUDGETS:
        print(f"\n{title}, budget de {budget} € par mois, τ = {TAU} (espérance S3 nette par an, comptée à S vrai)")
        print("estimateur    dépense/mois  mois au plafond   annoncé    à S vrai   écart à S connu   écart à la production")
        rows = {name: simulate(tables[name], G, budget, YEARS) for name in tables}
        for name, r in rows.items():
            print(f"{name:12s}  {r['spend']:7.1f} €      {r['capped']:6.1%}      {r['announced']:+7.0f} €   {r['real']:+7.0f} €    "
                  f"{r['real'] - rows['oracle']['real']:+7.0f} €         {r['real'] - rows['prod_8']['real']:+7.0f} €")
        out[budget] = rows
    return out


def bootstrap(tables, keys, games, mask, seed, title, names):
    """Intervalle à 95 % : semaines ISO tirées avec remise (grilles d'une même semaine ensemble), mêmes arrivées pour tous."""
    rng = np.random.default_rng(seed)
    weeks = defaultdict(list)
    for i in np.flatnonzero(mask):
        weeks[keys[i][2]].append(i)
    labels = list(weeks)
    draws = {name: [] for name in names}
    for b in range(BOOTSTRAP):
        picked = np.concatenate([weeks[labels[j]] for j in rng.integers(0, len(labels), len(labels))])
        idx = {k: picked[games[picked] == k] for k in (7, 8)}
        G = arrivals(idx, rng, BOOT_YEARS)
        rows = {name: simulate(tables[name], G, MAIN_BUDGET, BOOT_YEARS)["real"] for name in names}
        for name in names:
            draws[name].append(rows[name])
        if (b + 1) % 100 == 0:
            rate = (time.time() - START) / (b + 1)
            log(f"{title} : bootstrap {b + 1} / {BOOTSTRAP}")
    print(f"\n{title}, IC 95 % des écarts à {MAIN_BUDGET} € par mois (bootstrap de semaines)")
    print("estimateur     écart à S connu              écart à la production")
    out = {}
    for name in names:
        a = np.array(draws[name]) - np.array(draws["oracle"]); c = np.array(draws[name]) - np.array(draws["prod_8"])
        out[name] = {"to_oracle": np.percentile(a, [2.5, 50, 97.5]).tolist(), "to_prod": np.percentile(c, [2.5, 50, 97.5]).tolist()}
        print(f"{name:12s}   [{np.percentile(a, 2.5):+6.0f} ; {np.percentile(a, 97.5):+6.0f}] €        [{np.percentile(c, 2.5):+6.0f} ; {np.percentile(c, 97.5):+6.0f}] €")
    return out


def ticket_table(results, keys, mask, title):
    print(f"\n{title} : tickets par grille au seuil τ = {TAU}, sans budget")
    print("estimateur    tickets ≥ τ à Ŝ   vraiment ≥ τ   faux positifs   vrais manqués   écart |Δ| > 2 tickets   EV à S vrai des retenus")
    picked = [k for k, m in zip(keys, mask) if m]
    truth = [results[k]["oracle"] for k in picked]
    for name in results[picked[0]]:
        rows = [results[k][name] for k in picked]
        chosen = [a >= TAU for a, _ in rows]
        above = np.array([c.sum() for c in chosen], dtype=float)
        hits = np.array([(r >= TAU)[c].sum() for c, (_, r) in zip(chosen, rows)], dtype=float)
        ideal = np.array([(a >= TAU).sum() for a, _ in truth], dtype=float)
        evs = np.concatenate([r[c] for c, (_, r) in zip(chosen, rows)])
        print(f"{name:12s}      {above.mean():6.2f}          {hits.mean():6.2f}         {(above - hits).mean():6.2f}          {np.maximum(ideal - hits, 0).mean():6.2f}"
              f"             {np.mean(np.abs(above - ideal) > 2):6.1%}                  {evs.mean() if len(evs) else 0:.2f}")


def elasticity(results, keys, rows_by_key, mask, title):
    """d ln EV / d ln S des tickets de production, sur les grilles où Ŝ s'écarte de S d'au moins 10 %."""
    print(f"\n{title} : élasticité de l'espérance S3 aux mises (d ln EV / d ln S, tickets de production)")
    print("groupe            grilles   médiane   p10     p90")
    for label, test in (("sans pactole", lambda r: not r["jackpot"]), ("avec pactole", lambda r: r["jackpot"] > 0)):
        values = []
        for k, m in zip(keys, mask):
            row = rows_by_key[k[:2]]
            gap = np.log(row["est"]["prod_8"] / row["S"])
            if m and test(row) and abs(gap) >= 0.1:
                a, r = results[k]["prod_8"]
                values.append(float(np.median(np.log(a / r))) / gap)
        if values:
            print(f"{label:16s}  {len(values):5d}     {np.median(values):+.2f}   {np.percentile(values, 10):+.2f}   {np.percentile(values, 90):+.2f}")


def journal_rows(rows):
    """Grilles du journal aux rapports connus : Ŝ de production, Ŝ de `reg` (archive puis grilles du journal, en walk-forward), S déduit,
    q du journal et dernière répartition. Base ouverte en lecture seule."""
    path = os.path.expanduser("~/.local/share/edge/edge.db")
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    out = []
    grids = db.execute("""select g.game, g.number, g.closes_at, g.jackpot, r.winners * r.unit_gain from lotofoot_grid g
                          join lotofoot_rank r on r.game = g.game and r.number = g.number and r.rank = 2 where r.winners > 0 order by g.closes_at""").fetchall()
    known = [r for r in rows if r["deduced"]]
    for game, number, closes, jackpot, mass in grids:
        code = int(game[2:]); close = when(closes)
        same = [r for r in known if r["game"] == code and r["known"] <= close - DECISION_LEAD]
        row = {"game": code, "close": close, "known": close + DECISION_LEAD, "day": close.astimezone(PARIS).weekday(), "jackpot": jackpot or 0.0,
               "deduced": mass / RANK2_SHARE, "level": np.log(tail_median([r["deduced"] for r in same], 16)), "comp": []}
        reg = float(np.exp(row["level"] + ridge(row, [r for r in same if r["level"] is not None], False)))
        item = {"game": game, "number": number, "day": row["day"], "jackpot": row["jackpot"], "S": row["deduced"], "reg": reg, "journal": None}
        known.append(row)
        reco = db.execute("select stakes_estimate, inputs from lotofoot_reco where game = ? and number = ? order by id desc limit 1", (game, number)).fetchone()
        if reco:
            inputs = json.loads(reco[1])
            fetch = db.execute("""select s.fetch_id from lotofoot_split s join lotofoot_fetch f on f.id = s.fetch_id
                                  where s.game = ? and s.number = ? and f.fetched_at <= ? group by s.fetch_id having count(*) = ?
                                  order by f.fetched_at desc limit 1""", (game, number, closes, code)).fetchone()
            split = db.execute("select q_home, q_draw, q_away from lotofoot_split where fetch_id = ? and game = ? and number = ? order by position", (fetch[0], game, number)).fetchall()
            last = [[x / sum(line) for x in line] for line in split]
            item.update(journal=reco[0], p=inputs["p"], q=inputs["q"], last=last)
        out.append(item)
    return out


def count_above(item, stakes, q, tickets):
    P, Q = np.array(item["p"]), np.array(q); code = int(item["game"][2:]); prm = PRM[code]
    g = {"game": code, "S": stakes, "jackpot": {"type": 1, "amount": item["jackpot"]}}
    M = BF.masses(g)
    Pt = (1 - SHRINK) * P + SHRINK * Q; Pt /= Pt.sum(axis=1, keepdims=True)
    T = B16.greedy(P, Q, stakes, M, prm, tickets, 1)
    return sum(lf_fast.ev_p(t, Pt, Q, stakes, M, prm["kappa"], prm["beta"], prm["phi"], low_mult=LOW_MULT)[0] >= TAU for t in T)


def journal_work(item):
    if item["journal"] is None:
        return item, None
    return item, [count_above(item, s, q, k) for k in (16, 64) for s, q in ((item["journal"], item["q"]), (item["reg"], item["q"]), (item["S"], item["q"]), (item["S"], item["last"]))]


def journal_table(items):
    print("\nGrilles du journal (S déduit des rapports, environ 2 % au-dessus de S publié)")
    print("grille       jour  pactole    Ŝ journal    Ŝ reg      S déduit    journal/S − 1   reg/S − 1   tickets ≥ 1,7 sur 16 : Ŝ, reg, S, S + dernier q ; sur 64 : idem")
    days = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"]
    for item, counts in items:
        journal = item["journal"]
        print(f"{item['game']} n°{item['number']:<4d} {days[item['day']]}  {item['jackpot']:7.0f}   "
              f"{journal if journal else 0:9.0f}   {item['reg']:9.0f}   {item['S']:9.0f}   "
              + (f"{journal / item['S'] - 1:+6.1%}" if journal else "   —  ") + "     "
              + f"{item['reg'] / item['S'] - 1:+6.1%}" + "         "
              + (", ".join(f"{c:2d}" for c in counts[:4]) + " ; " + ", ".join(f"{c:2d}" for c in counts[4:]) if counts else "—"))


if __name__ == "__main__":
    reveal = "--test" in sys.argv
    every = [g for g in json.load(open("../data/lf/grids_fdj_all.json")) if g["game"] in (7, 8)]
    log(f"{len(every)} grilles archivées, {len(NAMES)} estimateurs, décision à la clôture − 8 h")
    rows = series(every, competitions(every))
    estimate(rows)
    scored = [r for r in rows if r["est"]]
    print(f"grilles évaluées : {len(scored)} sur {len(rows)} ({sum(r['train'] for r in scored)} d'entraînement, {sum(not r['train'] for r in scored)} de test)")
    train = [r for r in scored if r["train"]]; test = [r for r in scored if not r["train"]]
    error_table(train, "Entraînement : erreur sur S (Ŝ / S − 1)")
    detail_table(train, "Entraînement")
    score = {name: float(np.mean([abs(np.log(r["est"][name] / r["S"])) for r in train])) for name in NAMES}
    best = min(score.values())
    chosen = next(name for name in NAMES if score[name] <= best + 0.01)
    print(f"\nEstimateur retenu sur l'entraînement : {chosen} (|log| {score[chosen]:.3f}, meilleur {best:.3f} ; {len(NAMES)} configurations)")
    if reveal:
        error_table(test, "Test : erreur sur S (Ŝ / S − 1)")
        detail_table(test, "Test")

    matched = [g for g in json.load(open("../data/lf/grids_fdj.json")) if g["game"] in (7, 8)]
    by_key = {(r["game"], r["gid"]): r for r in rows}
    usable = []
    for g in matched:
        row = by_key.get((g["game"], g["gid"]))
        if row and row["est"]:
            usable.append({**g, "estimates": {**row["est"], "oracle": row["S"]}})
    log(f"{len(usable)} grilles appariées sur {len(matched)} ont un Ŝ")
    results = effects(usable)
    keys = sorted(results, key=lambda k: by_key[k]["close"])
    labels = [(*k, tuple(by_key[k]["close"].isocalendar()[:2])) for k in keys]
    games = np.array([k[0] for k in keys]); is_train = np.array([by_key[k]["train"] for k in keys])
    names = ["oracle"] + NAMES
    tables = {name: tables_for(results, keys, name) for name in names}
    print(f"\nGrilles appariées : {is_train.sum()} d'entraînement, {(~is_train).sum()} de test")
    ticket_table(results, keys, is_train, "Entraînement")
    elasticity(results, keys, by_key, is_train, "Entraînement")
    out = {"train": budget_table(tables, keys, games, is_train, 1, "Entraînement")}
    log("bootstrap de l'entraînement")
    out["train_ci"] = bootstrap(tables, labels, games, is_train, 3, "Entraînement", names)
    if reveal:
        ticket_table(results, keys, ~is_train, "Test")
        elasticity(results, keys, by_key, ~is_train, "Test")
        out["test"] = budget_table(tables, keys, games, ~is_train, 2, "Test")
        log("bootstrap du test")
        out["test_ci"] = bootstrap(tables, labels, games, ~is_train, 4, "Test", names)
        log("grilles du journal")
        items = journal_rows(rows)
        with Pool(8) as pool:
            journal_table(pool.map(journal_work, items))
    out["chosen"] = chosen
    json.dump(out, open(OUT, "w"))
    log(f"fini, résultats dans {OUT}")
