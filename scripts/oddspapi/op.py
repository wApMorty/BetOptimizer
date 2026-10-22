# Client minimal OddsPapi v4 (lecture seule). La clé est lue dans ~/.config/edge/oddspapi.key et n'est jamais affichée.
# Chaque réponse est conservée telle quelle dans oddspapi/ (cache : un même appel n'est jamais refait).
import json, os, time, hashlib, urllib.request, urllib.parse, urllib.error
BASE = "https://api.oddspapi.io/v4/"
KEY = open(os.path.expanduser("~/.config/edge/oddspapi.key")).read().strip()
DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "oddspapi")
_last = {}
def get(endpoint, cooldown=1.1, **params):
    name = endpoint + "_" + hashlib.sha1(json.dumps(params, sort_keys=True).encode()).hexdigest()[:12] + ".json"
    path = os.path.join(DIR, name)
    if os.path.exists(path):
        return json.load(open(path))
    wait = _last.get(endpoint, 0) + cooldown - time.time()
    if wait > 0: time.sleep(wait)
    q = urllib.parse.urlencode({**params, "apiKey": KEY})
    req = urllib.request.Request(BASE + endpoint + "?" + q, headers={"User-Agent": "edge-study/0.1"})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                body = json.load(r)
            break
        except urllib.error.HTTPError as e:
            msg = e.read().decode(errors="replace")[:300]
            if e.code == 429 and "REQUEST_LIMIT" not in msg:
                time.sleep(6 * (attempt + 1)); continue
            raise RuntimeError(f"{endpoint} {params}: HTTP {e.code} {msg}") from None
    _last[endpoint] = time.time()
    json.dump({"params": params, "fetched_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "body": body}, open(path, "w"))
    return {"params": params, "body": body}
def account():
    # Endpoint non décompté mais limité en débit : 3 s entre deux appels. L'URL (qui porte la clé) n'est jamais affichée.
    wait = _last.get("account", 0) + 3 - time.time()
    if wait > 0: time.sleep(wait)
    q = urllib.parse.urlencode({"apiKey": KEY})
    try:
        with urllib.request.urlopen(urllib.request.Request(BASE + "account?" + q, headers={"User-Agent": "edge-study/0.1"}), timeout=30) as r:
            body = json.load(r)
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"account: HTTP {e.code}") from None
    _last["account"] = time.time()
    return body
def used():
    return [x for x in account()["subscriptions"] if x["is_active"]][0]["request_count"]
