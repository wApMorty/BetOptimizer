# Probabilités sans marge (méthode puissance) du dernier relevé Pinnacle de notre archive, pour une liste de matchs.
import sqlite3, os
DB = os.path.expanduser("~/.local/share/edge/edge.db")
def power(odds):
    inv = [1 / x for x in odds]; lo, hi = 0.5, 3.0
    for _ in range(100):
        k = (lo + hi) / 2; s = sum(v ** k for v in inv); lo, hi = (k, hi) if s > 1 else (lo, k)
    return [v ** k for v in inv]
DBS = [DB, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "edgehome", "edge.db")]
def latest(home, away, book="pinnacle"):
    for db in DBS:
        r = _latest(db, home, away, book)
        if r: return r
    return None
def _latest(db, home, away, book):
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    row = con.execute("""select e.api_call_id, e.event_id, e.commence_time from event_seen e
        where e.home_team_raw=? and e.away_team_raw=? and exists (select 1 from quote q where q.api_call_id=e.api_call_id
        and q.event_id=e.event_id and q.bookmaker_key=? and q.market_key='h2h') order by e.api_call_id desc limit 1""",
        (home, away, book)).fetchone()
    if not row: return None
    cid, eid, ko = row
    pr = dict(con.execute("select outcome_raw, price from quote where api_call_id=? and event_id=? and bookmaker_key=? and market_key='h2h'",
                          (cid, eid, book)).fetchall())
    called = con.execute("select called_at from api_call where id=?", (cid,)).fetchone()[0]
    odds = [pr[home], pr["Draw"], pr[away]]
    return {"odds": odds, "p": power(odds), "overround": sum(1 / x for x in odds), "ko": ko, "at": called}
