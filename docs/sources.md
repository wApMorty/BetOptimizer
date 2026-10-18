# Sources et vérifications

Chaque fait indique sa **nature** : *test réel* (appel effectué avec la clé du projet) ou *doc* (lu dans la
documentation, non éprouvé). Date de toutes les vérifications ci-dessous : **2026-10-18**, relevés réalisés entre
10:57 et 11:05 UTC. Les réponses brutes sont dans le scratchpad de session, pas dans le dépôt.

## 1. The Odds API — offre gratuite

| Fait | Nature | Source |
|---|---|---|
| 500 crédits par mois, tous sports, « most bookmakers », tous marchés | doc | https://the-odds-api.com/ |
| Une clé fait 32 caractères ; compte neuf au premier appel (`x-requests-used: 0`) | test réel | `GET /v4/sports` |
| Date de réinitialisation du quota **non documentée** pour l'offre gratuite (la page ne la donne que pour les offres payantes : « même jour du mois que le début de l'abonnement ») | doc | https://the-odds-api.com/ |
| Historique des cotes : réservé aux offres payantes | doc | https://the-odds-api.com/liveapi/guides/v4/ |
| Stockage illimité des données **autorisé** (« Storing our data and retaining it indefinitely » figure dans « Permitted uses ») ; revente comme flux brut interdite ; attribution non requise | doc | https://the-odds-api.com/terms-and-conditions.html |

## 2. Clés des ligues (`GET /v4/sports?all=true`, gratuit, 180 sports renvoyés)

Toutes `active: true` et `has_outrights: false` au 2026-10-18 — *test réel*.

| Compétition | Clé API | Titre API |
|---|---|---|
| Premier League | `soccer_epl` | EPL |
| La Liga | `soccer_spain_la_liga` | La Liga - Spain |
| Bundesliga | `soccer_germany_bundesliga` | Bundesliga - Germany |
| Serie A | `soccer_italy_serie_a` | Serie A - Italy |
| Ligue 1 | `soccer_france_ligue_one` | Ligue 1 - France |
| Premiership écossaise | `soccer_spl` | Premiership - Scotland |

Pièges de nommage : la Championship est `soccer_efl_champ` (pas `soccer_england_championship`) ; la 2. Bundesliga
est `soccer_germany_bundesliga2` ; l'Écosse est `soccer_spl`, pas `soccer_scotland_*`.

Matchs à venir (`GET /v4/sports/{clé}/events`, coût 0, *test réel*) : EPL 25 (jusqu'au 2 nov.), La Liga 16
(26 oct.), Bundesliga 11 (25 oct.), Serie A 16 (25 oct.), Ligue 1 14 (25 oct.), Premiership 5 (24 oct.).

## 3. Bookmakers

Source : https://the-odds-api.com/sports-odds-data/bookmaker-apis.html (*doc*), confirmé par appel (*test réel*).

- Région `eu` (27 clés) : contient `pinnacle`, `winamax_fr`, `betclic_fr`, `unibet_fr`, `pmu_fr`, `winamax_de`…
- Région `fr` (5 clés) : `betclic_fr`, `netbet_fr`, `pmu_fr`, `unibet_fr`, `winamax_fr`. Seule `netbet_fr` est propre à `fr`.
- `pinnacle` est **uniquement** dans `eu`. Mention de la page : « Odds are from public website which may incur a delay ».
- `zebet` et `parionssport_fr` n'apparaissent dans aucune des deux régions.
- Dans les réponses, `title` vaut `Winamax (FR)`, `Betclic (FR)`, `Pinnacle`. L'ordre des bookmakers dans la réponse
  n'est pas celui de la requête.

### Couverture mesurée (marché `h2h`, `bookmakers=pinnacle,winamax_fr,betclic_fr`) — *test réel*

| Ligue | Matchs | avec Pinnacle | avec Winamax | avec Betclic | les trois |
|---|---|---|---|---|---|
| `soccer_epl` | 25 | 15 | 25 | 25 | 15 |
| `soccer_spain_la_liga` | 16 | 16 | 16 | 16 | 16 |
| `soccer_germany_bundesliga` | 11 | 11 | 11 | 11 | 11 |
| `soccer_italy_serie_a` | 16 | 16 | 16 | 16 | 16 |
| `soccer_france_ligue_one` | 14 | 14 | 14 | 14 | 14 |
| `soccer_spl` | 5 | 5 | **0** | **0** | 0 |

- **Écosse : ni Winamax ni Betclic.** Avec `regions=fr`, la Premiership est cotée par `unibet_fr`, `pmu_fr` et
  `netbet_fr` (5 matchs sur 5 chacun), pas par Winamax ni Betclic. Un seul relevé : à réévaluer avec l'archive.
- **Horizon de Pinnacle.** Pinnacle apparaît pour les matchs à ≤ 201 h (8,4 jours) du coup d'envoi et manque pour
  ceux à ≥ 313 h (13 jours) ; l'intervalle intermédiaire n'a pas été observé. Les 10 matchs EPL sans Pinnacle sont
  tous au-delà de 313 h. Winamax et Betclic cotent dès le plus lointain.

## 4. Décompte des crédits

| Fait | Nature |
|---|---|
| Formule : coût = nombre de marchés × nombre de régions (« 1 per region per market ») | doc |
| « Every group of 10 bookmakers is the equivalent of 1 region » : jusqu'à 10 bookmakers = 1, de 11 à 20 = 2 | doc |
| `bookmakers` prime sur `regions` quand les deux sont fournis | doc |
| 1 marché (`h2h`) × 3 bookmakers, sans `regions` : **1 crédit** (`x-requests-last: 1`) pour chacune des 6 ligues | test réel |
| **Un appel renvoie tous les matchs à venir de la ligue** (25 pour l'EPL) pour le même coût : le coût ne dépend pas du nombre de matchs | test réel |
| `regions=fr` × 1 marché (3 bookmakers renvoyés pour la Premiership) : 1 crédit | test réel |
| Gratuits : `/sports`, `/events` | doc + test réel |
| Une réponse sans événement ne coûte rien | doc (non éprouvé) |
| Au-delà de 10 bookmakers : 2 crédits | doc (non éprouvé) |
| **Un bookmaker inconnu** (`bookmakers=bookmaker_bidon`) : HTTP 200, chaque match renvoyé avec `"bookmakers": []`, **coût 1 crédit** — une faute de frappe est facturée sans erreur | test réel |
| Erreurs 401 (clé), 404 (sport), 422 (marché) : coût 0 | test réel |

## 5. En-têtes de quota (*doc* + *test réel*)

- `x-requests-remaining` : crédits restants jusqu'à la réinitialisation.
- `x-requests-used` : crédits consommés depuis la dernière réinitialisation.
- `x-requests-last` : coût du dernier appel.
- Présents sur les réponses 200, 404 et 422, **absents sur 401** (clé invalide). `remaining + used = 500`.
- Un appel à `/sports` (gratuit) renvoie ces en-têtes : il permet de lire le quota réel **sans rien dépenser**.

## 6. Format de réponse `GET /v4/sports/{clé}/odds` (*test réel*)

Paramètres utilisés : `markets=h2h&bookmakers=pinnacle,winamax_fr,betclic_fr&oddsFormat=decimal&dateFormat=iso`
(`decimal` et `iso` sont les valeurs par défaut). Filtres de date : `commenceTimeFrom`, `commenceTimeTo`
(ISO 8601, *doc*).

```json
[{
  "id": "dfd5d6648be46426e418801584d46a68",
  "sport_key": "soccer_epl",
  "sport_title": "EPL",
  "commence_time": "2026-10-18T13:00:00Z",
  "home_team": "Bournemouth",
  "away_team": "Sunderland",
  "bookmakers": [{
    "key": "winamax_fr",
    "title": "Winamax (FR)",
    "last_update": "2026-10-18T10:57:26Z",
    "markets": [{
      "key": "h2h",
      "last_update": "2026-10-18T10:57:25Z",
      "outcomes": [
        {"name": "Bournemouth", "price": 2.1},
        {"name": "Sunderland",  "price": 3.15},
        {"name": "Draw",        "price": 3.3}
      ]
    }]
  }]
}]
```

Mesuré sur 87 matchs et 241 marchés `h2h` :

- Toujours 3 issues ; la 3ᵉ s'appelle littéralement `"Draw"` ; les deux autres portent le **nom exact** de
  `home_team` / `away_team`. Aucun autre nom d'issue observé.
- **L'ordre des issues n'est pas domicile / extérieur / nul.** Il suit l'ordre alphabétique des deux équipes, puis
  `Draw` (ex. Udinese–Lecce : `Lecce`, `Udinese`, `Draw`). Le domicile est en première position dans 138 cas sur 241
  seulement. Un parseur positionnel serait faux dans 43 % des cas : il faut associer par nom.
- `last_update` existe au niveau bookmaker **et** au niveau marché ; la doc déclare celui du bookmaker déprécié :
  utiliser celui du marché.
- Cotes décimales de 1,15 à 11,44. Somme des probabilités implicites de 1,027 à 1,167 (dans [1,00 ; 1,30]).
- Noms d'équipes bruts variables d'une source à l'autre (`Paris Saint Germain`, `Inter Milan`, `Falkirk F.C.`,
  `Brighton and Hove Albion`, `Borussia Monchengladbach`, `Málaga`) : la table d'alias est indispensable en phase 2.
- **Matchs déjà commencés.** Udinese–Lecce (coup d'envoi 10:31Z) figurait dans la réponse de 10:58Z avec un
  `last_update` de 10:58 : ces cotes sont très probablement des cotes en direct (non prouvé). Elles ne doivent pas
  entrer dans une archive d'avant-match.

## 7. Erreurs (*test réel*)

| Cas | HTTP | Corps |
|---|---|---|
| Clé invalide | 401 | `{"message":"API key is not valid…","error_code":"INVALID_KEY","details_url":"…"}` (aucun en-tête `x-requests-*`) |
| Sport inconnu | 404 | `{"message":"Unknown sport","error_code":"UNKNOWN_SPORT",…}` |
| Marché invalide | 422 | `{"message":"Invalid markets: …","error_code":"INVALID_MARKET",…}` |
| Limite de débit | 429 | doc uniquement : « try spacing out requests over several seconds » (non provoqué) |

Les corps d'erreur sont du JSON avec `error_code` : à lire pour construire le message d'erreur.
Liste officielle des codes : https://the-odds-api.com/liveapi/guides/v4/api-error-codes.html (non lue).

## 8. Crates (crates.io, versions stables au 2026-10-18)

| Crate | Version | MSRV déclaré | Publiée |
|---|---|---|---|
| clap | 4.6.7 | 1.85 | 2026-09-14 |
| serde | 1.0.229 | 1.56 | 2026-07-18 |
| serde_json | 1.0.154 | 1.71 | 2026-10-11 |
| toml | 1.1.8 | 1.85 | 2026-10-09 |
| ureq | 3.4.2 | 1.85 | 2026-09-13 |
| rusqlite | 0.40.2 | non déclaré | 2026-08-08 |
| chrono | 0.4.45 | 1.62 | 2026-06-04 |
| chrono-tz | 0.10.4 | 1.65 | 2025-07-11 |
| directories | 6.0.0 | non déclaré | 2025-01-12 |
| tracing | 0.1.44 | 1.65 | 2025-12-18 |
| tracing-subscriber | 0.3.23 | 1.65 | 2026-03-13 |
| thiserror | 2.0.21 | 1.77 | 2026-09-23 |
| anyhow | 1.0.104 | 1.68 | 2026-07-18 |
| approx | 0.5.1 | non déclaré | 2022-01-12 |

- Chaîne Rust minimale : **1.85** (édition 2024 et plusieurs dépendances).
- `ureq` 3.4.2 — API lue dans les sources le 2026-10-18 (`~/.cargo/registry/src/*/ureq-3.4.2`) :
  `Agent::config_builder()…build()` puis `Agent::new_with_config(config)` ; `http_status_as_error(false)` (par
  défaut `true` : un 4xx/5xx devient une erreur sans corps lisible) ; `max_redirects(u32)`, `https_only(bool)`,
  `timeout_global(Option<Duration>)` ; `agent.get(url).call()` → `Result<http::Response<Body>, ureq::Error>` ;
  `response.status()`, `response.headers()` (type `http::HeaderMap`, ré-exporté par `ureq::http`) ;
  `response.body_mut().read_to_string()` (limite par défaut de 10 Mo). Features par défaut : `rustls` (crypto
  `ring`, compile avec un simple `gcc`) et `gzip`.
- `ureq::Error::BadUri(String)` recopie l'URI fautive, donc la clé API : toute erreur `ureq` est expurgée avant
  affichage (`crates/core/src/ingest/client.rs`).
- `rusqlite` : feature `bundled` présente ; `STRICT` et les triggers fonctionnent avec le SQLite embarqué.

## 9. Environnement local au 2026-10-18

- `cargo`, `rustc`, `rustup` : **absents**. `sqlite3` 3.53.4, `curl`, `jq`, `python3` 3.14.7 présents.
- systemd 262 ; lingering désactivé à la vérification, **activé ensuite** (`loginctl enable-linger`, `Linger=yes`).
- `.env` à la racine du dépôt : permissions 644 à la vérification, **corrigées en 600** ; `.gitignore` créé
  (`.env`, `target/`, `*.db`). La variable `EDGE_ODDS_API_KEY` n'était pas exportée dans l'environnement du shell de
  session ; la vérification a lu la valeur dans `.env` sans jamais l'afficher ni l'écrire ailleurs.
- Compilateur C (`gcc`) présent : suffisant pour `rusqlite` (`bundled`) et `ureq` (rustls).

## 10. Crédits consommés

| Appels | Crédits |
|---|---|
| `/sports`, 6 × `/events`, 3 erreurs (401, 404, 422) | 0 |
| 6 × `/odds` (une par ligue, `h2h`, 3 bookmakers) | 6 |
| 1 × `/odds` avec un bookmaker inconnu (non prévu) | 1 |
| 1 × `/odds` Premiership, `regions=fr` | 1 |
| Premier relevé réel de l'archive (`edge snapshot`, 2026-10-18 12:03 UTC) : 6 × `/odds` | 6 |
| **Total** | **14** (reste 486 au dernier en-tête lu) |

## 11. Premier relevé réel (2026-10-18, 12:03 UTC, 6 bookmakers demandés)

Bookmakers demandés : `pinnacle, winamax_fr, betclic_fr, unibet_fr, pmu_fr, netbet_fr` (6 ≤ 10 : 1 crédit par
appel, confirmé : 6 appels = 6 crédits). Résultat : 85 matchs à venir, 1 284 cotes, 2 matchs déjà commencés ignorés.

| Ligue | Matchs | Remarque |
|---|---|---|
| `soccer_epl` | 25 | Pinnacle sur 15 matchs seulement (horizon, voir section 3) |
| `soccer_spain_la_liga` | 15 (+1 commencé) | |
| `soccer_germany_bundesliga` | 11 | |
| `soccer_italy_serie_a` | 15 (+1 commencé) | |
| `soccer_france_ligue_one` | 14 | |
| `soccer_spl` | 5 | Pinnacle, Unibet, PMU, NetBet ; ni Winamax ni Betclic |

`netbet_fr` ne couvre que 5 à 9 matchs par ligue (les prochains) ; les autres bookmakers couvrent tous les matchs
de leur ligue.

**Marge moyenne observée** (somme des probabilités implicites 1/cote d'un 1X2, moins 1 ; premier relevé, 34 à 84
matchs par bookmaker). Observation sur un seul relevé, pas une conclusion : à refaire quand l'archive sera plus
épaisse.

| Bookmaker | Marge moyenne | Min – max |
|---|---|---|
| `pinnacle` | 4,7 % | 2,7 % – 6,5 % |
| `pmu_fr` | 8,7 % | 7,8 % – 12,1 % |
| `betclic_fr` | 9,6 % | 6,8 % – 14,1 % |
| `netbet_fr` | 10,2 % | 8,5 % – 13,0 % |
| `unibet_fr` | 11,9 % | 8,1 % – 18,2 % |
| `winamax_fr` | 14,2 % | 8,7 % – 16,7 % |

## 12. Phase 2 — football-data.co.uk (vérifié le 2026-10-18)

Sources : fichiers CSV téléchargés (66 : 6 championnats × saisons 2016/17 à 2026/27, tous HTTP 200),
https://football-data.co.uk/notes.txt, `matches.php` et `data.php`.

**Accès.** `https://football-data.co.uk/mmz4281/{SSSS}/{Div}.csv` avec `SSSS` = `2526` pour 2025/26 et `Div` parmi
`E0` (Premier League), `SP1` (La Liga), `D1` (Bundesliga), `I1` (Serie A), `F1` (Ligue 1), `SC0` (Premiership).
L'adresse avec `www.` renvoie un 302 vers le domaine nu. En-tête `x-ws-ratelimit-limit: 1000` ; le site a répondu
**429** après environ 70 requêtes rapprochées : pas de rafale, au moins 1 s entre deux requêtes, cache sur disque,
et seule la saison courante est retéléchargée en routine.

**Format.**
- Virgule comme séparateur, BOM UTF-8 en tête de fichier, lignes vides possibles (filtrer sur `HomeTeam`).
  `SC0_1819.csv` n'est pas en UTF-8 (octet 0xA0) : décoder en Latin-1 quand l'UTF-8 échoue.
- Dates `dd/mm/yy` en 2016/17, `dd/mm/yyyy` ensuite.
- `Time` est l'**heure du Royaume-Uni** pour toutes les ligues (Bundesliga 14:30 = 15:30 CET ; Serie A 19:45 =
  20:45 CET) : conversion Europe/London → UTC nécessaire.
- Le nombre de colonnes varie : 65 (2016/17), 132 (2025/26), 114 (2026/27). Lire par nom, jamais par position.
- Volume : 20 168 matchs terminés sur 2016/17–2025/26 (E0 3 800, SP1 3 800, I1 3 800, F1 3 477, D1 3 060,
  SC0 2 231) et 345 matchs de 2026/27 (jusqu'au 12/10/2026).

**Statistiques de match** (test sur E0 pour toutes les saisons) : buts (`FTHG`, `FTAG`), mi-temps, `Referee`, tirs
(`HS`, `AS`), cadrés (`HST`, `AST`), fautes, corners, cartons : 100 % remplis de 2016/17 à 2026/27.

**Cotes.**
- Depuis 2019/20, deux relevés : « pré-clôture » (week-end : vendredi après-midi, au plus tard 17:00 heure
  britannique ; semaine : mardi, au plus tard 13:00) et clôture (suffixe `C`, par exemple `B365CH`). Avant 2019/20,
  seulement la pré-clôture, mais la clôture Pinnacle (`PSC*`) existe : remplie à 100 % dès 2016/17 (le site annonce
  2012/13).
- `Avg*` et `Max*` (moyenne et maximum du marché, via Oddsportal depuis 2019/20) : 100 % depuis 2019/20.
- **Pinnacle est retiré progressivement du jeu de données.** Saison 2025/26, dernier match avec `PSH` : E0
  08/01/2026 (210 matchs sur 380), SP1 12/01 (189), D1 15/01 (150), I1 15/01 (200), F1 04/01 (153), SC0 26/10/2025
  (55 sur 228). Saison 2026/27 : 0 %. Aucune explication sur le site.
- Colonnes disparues en 2026/27 : Pinnacle (`PS*`, `P>2.5`, `PAH*`), Coral (`CL*`), Ladbrokes (`LB*`), BetMGM
  (`BMGM*`). Nouvelles : Paddy Power (`PP*`), Sky Bet (`SKB*`) et **`HxG` / `AxG`** (xG par match, 100 % des 345
  matchs de 2026/27, **source non documentée**).
- Conséquence : la référence de marché est Pinnacle sans marge jusqu'en janvier 2026 ; ensuite repli sur la moyenne
  de clôture `AvgC*`, dont l'écart à Pinnacle se mesure sur 2019/20–2025/26.

**Licence.** Aucune mention d'usage, de redistribution ni de licence trouvée sur `data.php` ni `matches.php`. Usage
personnel, cache local, aucune redistribution.

**Noms d'équipes.** 182 noms distincts dans les fichiers (E0 35, I1 34, F1 33, SP1 32, D1 31, SC0 17), avec des
formes courtes (`Man United`, `Nott'm Forest`, `Hull`). Sur les 106 noms de notre archive The Odds API, 61 sont
identiques à ceux de football-data et 45 sont à rapprocher (`AC Milan`, `Borussia Dortmund`, `Brighton and Hove
Albion`…) : la table d'alias est indispensable.

## 13. Phase 2 — sources de statistiques d'équipes et de joueurs (état des lieux au 2026-10-18)

| Source | Contenu | Couverture | Verdict |
|---|---|---|---|
| football-data.co.uk | tirs, cadrés, corners, fautes, cartons, arbitre ; xG dès 2026/27 | 6 ligues, 2016/17+ (xG : 2026/27 seulement) | **Base retenue** (niveau équipe) |
| Understat | xG par match, par joueur, par tir | Big 5 (+ Russie), 2014/15+ ; pas l'Écosse | **Écartée** : `robots.txt` = `User-agent: *` / `Disallow: /` (vérifié) |
| FBref | statistiques avancées Opta | — | **Écartée** : Sports Reference a retiré ces statistiques le 20 janvier (2026 selon les sources) après la rupture avec Opta / StatsPerform |
| API officielle Fantasy Premier League (`fantasy.premierleague.com/api/bootstrap-static/`) | 667 joueurs EPL : minutes, `expected_goals`, `expected_assists`, `status`, `news`, `chance_of_playing_next_round` | EPL seulement ; état courant | **Écartée le 2026-10-18** : accès techniquement possible (HTTP 200), mais les conditions du site de la Premier League limitent l'usage à un cadre « privé et personnel » et citent la création d'une base à partir de contenus téléchargés comme usage interdit ; aucune condition propre à l'API (non documentée). Pas d'archive. |
| ClubElo (`api.clubelo.com`) | Elo des clubs | Big 5, Écosse non confirmée | **Indisponible au test** (502 et délais dépassés). Elo recalculable depuis les résultats |
| API-Football (api-sports.io) | compositions, blessures, statistiques de joueurs | 100 requêtes par jour gratuites (source tierce) | **À tester avec une clé** ; couverture de l'Écosse non vérifiée |
| football-data.org (API) | résultats, calendrier | offre gratuite : 12 compétitions dont aucune écossaise, 10 requêtes par minute, ni statistiques de joueurs ni cotes (sources tierces) | Sans intérêt |

Aucune source gratuite et licite de statistiques de joueurs n'a été trouvée pour les cinq grands championnats hors
Premier League ni pour l'Écosse.

## 14. Phase 2 — mesures sur l'historique (rapport `edge market report`, 2026-10-18)

20 513 matchs importés (66 fichiers), 284 119 lignes de cotes ; 19 138 matchs avec une clôture Pinnacle.
Saisons incomplètes : Ligue 1 2019/20 (279 matchs sur 380) et Premiership 2019/20 (179 sur 228), arrêtées par le
Covid. Aucun résultat publié ne contredit les buts. Seules aberrations de cotes : l'exchange Betfair (`BFE`).

| Mesure | Résultat |
|---|---|
| Marge moyenne, clôture | Pinnacle 2,6 % ; moyenne du marché 4,9 % ; Bet365 5,7 % ; William Hill 6,2 % |
| Marge de Pinnacle par saison | 2,1 % (2016/17) à 3,1 % (2019/20 et 2024/25) : stable |
| Méthodes de retrait de marge (Pinnacle, clôture) | log loss multiplicative 0,9582, power 0,9580, shin 0,9580 : écart non significatif |
| Méthodes sur bookmakers à marge plus élevée | `power` et `shin` meilleurs que la multiplicative pour les cinq bookmakers testés ; significatif pour Bet&Win et Interwetten |
| Moyenne du marché contre Pinnacle (clôture, 12 959 matchs communs) | Δ log loss −0,00009 [−0,00047 ; +0,00028] : indiscernables |
| Avant-match contre clôture (Pinnacle) | Δ log loss +0,00232 [+0,00108 ; +0,00360] : l'avant-match est moins bon |
| Calibration | Points sur la diagonale dans les six ligues ; plus de bruit en Écosse (2 057 matchs) |
| Rendement à la cote brute, par cote juste | < 1,5 : −0,0 % ; 1,5–2 : −0,6 % ; 2–3 : −1,4 % ; 3–5 : −4,1 % ; 5–10 : −7,4 % ; ≥ 10 : −14,8 % (± 6,1) |

Méthode retenue : `power` (configuration `margin_method`).

À noter : la marge de Pinnacle lue en direct via The Odds API le 2026-10-18 (4,7 % en moyenne, premier relevé,
matchs jusqu'à 8 jours à l'avance) est supérieure à celle des fichiers football-data (2,9 % avant-match, 2,6 %
clôture). Écart à expliquer avec l'archive : horizon du match, délai du site public de Pinnacle, ou autre.

## 15. Phase 3 — vérifications avant les modèles (2026-10-18)

**Optimisation (`argmin`).** `argmin` 0.11.0 (2025-09-28) et `argmin-math` 0.5.1 (2025-10-02) ; la fonctionnalité
`vec` d'`argmin-math` (backend `Vec<f64>`) est active par défaut. Le problème s'exprime en implémentant deux
traits : `CostFunction { type Param; type Output; fn cost(&self, &Param) -> Result<Output, Error> }` et
`Gradient { type Param; type Gradient; fn gradient(&self, &Param) -> Result<Gradient, Error> }`. Solveur :
`LBFGS::new(linesearch, m)` avec `with_tolerance_grad` / `with_tolerance_cost` (lu dans les sources 0.11.0).
L'`Error` d'`argmin` est une erreur `anyhow` : on la convertit à la frontière de notre type d'erreur.

**Lois statistiques (`statrs`).** 0.19.1 (2026-08-11), **MSRV 1.89** : passer `rust-version` de 1.85 à 1.89 si la
crate est ajoutée.

**Référence (`penaltyblog` 1.13.1).** S'installe dans un venv jetable (Python 3.14.7, avec numpy 2.5.4, scipy
1.18.1, pandas 3.0.6). Conventions lues dans ses sources :
- `dixon_coles_weights(dates, xi=0.0018)` : poids `exp(−xi · jours avant la date de base)` ;
- paramètres : attaque (une par équipe), défense (une par équipe), avantage du terrain, `rho` ;
  `λ_dom = exp(avantage + attaque_dom + défense_ext)`, `λ_ext = exp(attaque_ext + défense_dom)` ;
- contrainte `Σ attaque = nombre d'équipes` ; bornes ±2,5 (équipes), [0 ; 2] (avantage), ±2,5 (`rho`) ;
  valeurs initiales attaque 1, défense −1, avantage 0,25, `rho` −0,1 ;
- correction des petits scores : `log(max(ε, 1 − ρ·λ·μ))` pour 0-0, `log(max(ε, 1 + ρ·λ))` pour 0-1,
  `log(max(ε, 1 + ρ·μ))` pour 1-0, `log(max(ε, 1 − ρ))` pour 1-1 ;
- **aucune régularisation** : la régularisation L2 du projet est un ajout.
Les `λ` et `μ` prédits sont invariants au choix de la contrainte (translater toutes les attaques de `c` et toutes
les défenses de `−c` ne change rien) : on les compare directement, avec le `rho` et l'avantage du terrain.

**Cotes plus/moins 2,5 buts dans football-data** (test sur E0, contrôlé sur les six ligues en 2024/25).
Colonnes `P>2.5`/`P<2.5` (Pinnacle), `Avg>2.5`/`Avg<2.5`, `B365>2.5`/`B365<2.5`, `Max…` à l'avant-match, et le même
jeu avec `C` pour la clôture (`PC>2.5`, `AvgC>2.5`, `B365C>2.5`…). Remplissage : 100 % de 2019/20 à 2026/27 pour la
moyenne et Bet365 ; Pinnacle 98–100 % jusqu'en 2024/25, 55 % en 2025/26, absent en 2026/27. Avant 2019/20 : seulement
les anciennes moyennes `BbAv>2.5`. Aucune cote « les deux équipes marquent » dans football-data : cette issue ne
peut être jugée que par sa calibration contre les résultats.

## 16. Phase 3 — mesures des modèles (`edge model report`, 2026-10-18)

Protocole : entraînement 2016/17–2023/24, validation 2024/25, test du 2025-07-01 au 2026-10-12 (2 325 matchs prédits
par chaque configuration, 0 prédiction avec ρ ramené dans le domaine faisable). 64 configurations essayées en une
session de réglage (Poisson 16, Dixon-Coles 48), environ 10 secondes par configuration (six fils d'exécution).

| Mesure | Résultat |
|---|---|
| Validation, meilleur Dixon-Coles | 0,98138 (ξ = 0,004, κ = 0,5, mélange de tirs 0,5) ; en bordure de grille |
| Validation, meilleur Poisson | 0,98251 (ξ = 0,002, κ = 2) |
| Validation, meilleur Dixon-Coles à buts seuls | 0,98256 (ξ = 0,002, κ = 2) |
| Test, moyenne du marché, clôture (référence) | 0,9744 [0,9578 ; 0,9917] |
| Test, moyenne du marché, avant-match | 0,9754 (Δ +0,0010 [−0,0015 ; +0,0036] : non significatif sur ce test) |
| Test, Dixon-Coles gelé | 0,9931 [0,9782 ; 1,0095] ; Δ vs marché **+0,0188 [+0,0130 ; +0,0251]**, ΔRPS +0,0049 |
| Test, Poisson | 0,9926 ; Δ +0,0183 |
| Test, Dixon-Coles à buts seuls | 0,9923 ; Δ +0,0180 : les tirs n'apportent rien démontrable |
| Test, fréquences passées de la ligue | 1,0713 : tous les modèles font nettement mieux |
| Test, matchs avec clôture Pinnacle (953) | Pinnacle 0,9699 ; moyenne du marché 0,9675 (Δ −0,0024 [−0,0045 ; −0,0003]) ; Dixon-Coles +0,0222 |
| Écart par ligue (Dixon-Coles moins marché) | Premier League +0,0052 [−0,0114 ; +0,0221] (non distinguable, 440 matchs) ; La Liga +0,0233 ; Bundesliga +0,0288 ; Serie A +0,0164 ; Ligue 1 +0,0182 ; Premiership +0,0247 |
| Plus de 2,5 buts | marché 0,6704 ; Dixon-Coles 0,6851 (Δ +0,0147 [+0,0092 ; +0,0209]) ; fréquence passée 0,6902 |
| Les deux équipes marquent | Dixon-Coles 0,6861 contre 0,6898 pour la fréquence passée (Δ −0,0037 [−0,0099 ; +0,0027], non significatif) |
| Test d'information, poids optimal du modèle | **0,00** avec la clôture ET avec l'avant-match : aucune information en plus du marché |
| Proxy de buts attendus (hors échantillon, xG publiés 2026/27) | ≈ 0,31 but par tir cadré, 0 par tir non cadré ; corrélation 0,59 à 0,76 selon la ligue ; rapport des moyennes 0,92 à 1,01 |

Référence de vérification : `penaltyblog` 1.13.1 sur un même jeu simulé : coût de notre optimum 411,2196 contre
411,2204 (le nôtre est au moins aussi bon), λ et μ à 0,43 %, ρ et avantage du terrain à 5·10⁻³.

## 17. Phase 4 — vérifications avant le backtest (2026-10-18)

Prototypes jetables en Python sur la base réelle (hors dépôt). Ils servent à orienter le plan, pas à conclure.

**Les cotes de l'archive ne sont pas des cotes finales.** Écart entre le relevé (`api_call.called_at`) et le coup d'envoi
(`event_seen.commence_time`) pour les 85 matchs du premier relevé : minimum 1 h, moyenne 137 h (près de 6 jours),
maximum 368 h. Aucun relevé n'est pris dans l'heure précédant un match. Les cotes de clôture n'existent que dans
l'historique football-data (Pinnacle jusqu'en janvier 2026, puis la moyenne du marché) ; en direct, elles
n'existeront qu'avec le relevé « juste avant le coup d'envoi » de la phase 5 (environ 1 crédit par ligue et par créneau).

**Prix mous contre Pinnacle au même instant (football-data, avant-match, pari sur l'issue de plus forte espérance,
espérance = cote × probabilité sans marge de Pinnacle − 1, seuil 0 %).** Rendement à mise fixe (± erreur-type) et CLV moyen
(cote prise × probabilité de clôture de Pinnacle − 1) :

| Bookmaker | Entraînement 2016–2023 | Validation 2024/25 | Test 2025/26+ |
|---|---|---|---|
| Bet365 | 1 652 mises, −0,8 % ± 4,4, CLV +0,1 % | 119, +0,1 % ± 14,2, CLV −0,2 % | 40, −10,8 % ± 28,5, CLV −1,4 % |
| William Hill | 1 216, +3,1 % ± 4,7, CLV +0,1 % | 114, +2,9 % ± 10,0 | 0 |
| Bet&Win | 1 186, +0,5 % ± 4,1, CLV +0,3 % | 105, +3,7 % ± 9,1 | 67, −16,2 % ± 11,1, CLV −1,7 % |
| **Interwetten** | **3 028, +7,5 % ± 2,4, CLV +2,3 %** (seuil 3 % : 894, +18,7 % ± 5,8, CLV +5,9 %) | 0 | 0 |
| VC Bet | 1 671, −10,3 % ± 5,0 | 0 | 0 |

Interprétation prudente : trois bookmakers sur quatre ne battent pas Pinnacle au même instant (rendement et CLV proches
de zéro). Interwetten est un bookmaker **lent** (ses prix retardent sur Pinnacle, et la clôture lui donne raison), mais il
disparaît des données après 2022/23 et n'est pas agréé en France : le mécanisme existe, pas une stratégie utilisable.
Les échantillons de validation et de test sont trop petits (40 à 119 mises) pour confirmer par le rendement un
avantage inférieur à environ 15 % : seul le CLV, de variance bien plus faible, peut trancher.

**Valeur actuelle chez les opérateurs français (un seul relevé, 75 matchs avec Pinnacle).** Meilleure espérance par match
contre Pinnacle sans marge (méthode power) : Winamax −8,4 % (aucun match positif), Betclic −5,6 % (2 matchs positifs,
maximum +0,9 %), Unibet −7,1 %, PMU −4,2 % (3 matchs positifs, maximum +2,4 %), NetBet −6,6 %. Aucune espérance ≥ 3 %.

**Décote française provisoire** (rapport moyen des cotes bookmaker / Pinnacle, un relevé) : Winamax ×0,910, Betclic ×0,957,
Unibet ×0,934, PMU ×0,958, NetBet ×0,937 (écart-type de 3 à 6 % selon le bookmaker). À réestimer avec l'archive.

**Sélection « EV+ » par le modèle (prototype, prédictions walk-forward déjà enregistrées, prix Bet365 d'avant-match).**
Pour chaque match, pari sur l'issue de plus forte espérance selon le Dixon-Coles gelé, si elle dépasse un seuil. Valeurs
illustratives, aucun seuil ajusté :

| Seuil (EV modèle) | Validation 2024/25 | Test 2025/26+ |
|---|---|---|
| ≥ 0 % | 1 648 mises, ROI −9,9 % ± 4,4, CLV −7,4 % ; gagnées 25,4 % (modèle : 31,4 % ; clôture du marché : 26,2 %) | 1 962 mises, ROI −12,4 % ± 3,9, CLV −7,4 % ; gagnées 25,5 % (modèle 32,5 % ; marché 27,0 %) |
| ≥ 5 % | 1 205 mises, ROI −8,8 %, CLV −7,9 % ; gagnées 23,4 % (modèle 29,9 % ; marché 23,8 %) | 1 447 mises, ROI −12,4 %, CLV −8,0 % ; gagnées 23,4 % (modèle 31,2 % ; marché 24,6 %) |
| ≥ 12 % | 759 mises, ROI −9,1 %, CLV −8,3 % | 921 mises, ROI −17,4 % ± 6,2, CLV −8,5 % ; gagnées 19,5 % (modèle 29,3 % ; marché 21,7 %) |

Lecture : les matchs que le modèle juge « à valeur » sont ceux où il s'écarte le plus du marché (surtout des nuls et des
outsiders). Sur ce sous-ensemble, la fréquence réelle suit la probabilité de **clôture du marché** (≈ 24 %), pas celle
du modèle (≈ 30 %) ; plus le seuil monte, plus l'écart s'aggrave. Les écarts du modèle au marché sont donc de l'erreur,
pas de l'information (effet de sélection : on retient les plus grandes erreurs du modèle). Cohérent avec le poids
optimal nul du test d'information. Le raisonnement vaut pour ce modèle (buts, forces d'équipe) et ces prix, pas pour
toute source d'avantage.

## 18. Phase 4 — mesures du backtest (`edge backtest report`, 2026-10-18)

**Protocole.** Huit règles de sélection : sept bookmakers de football-data contre Pinnacle sans marge au même relevé
(Bet365, William Hill, Bet&Win, Interwetten, VC Bet, Ladbrokes, 1xBet) et le Dixon-Coles gelé contre le prix de Bet365.
Une issue au plus par match ; décision sur les cotes d'avant-match, CLV mesuré sur la clôture de Pinnacle (à défaut, la
moyenne du marché). Seuils essayés : 0, 1, 2, 3, 5, 8 % ; critère : CLV moyen le plus haut sur entraînement + validation,
au moins 50 paris. **Une seule session de réglage, 54 configurations**, puis gel avant toute lecture du test.

| Règle | Seuil gelé | CLV au réglage (paris) | CLV au test (paris) | Verdict |
|---|---|---|---|---|
| Bet365 | 3 % | +0,88 % (297) | −0,22 % [−5,4 ; +5,1] (8) | test sans puissance |
| William Hill | 5 % | +2,71 % (68) | — (0) | sans puissance |
| Bet&Win | 3 % | +2,15 % (73) | −8,89 % [−25,1 ; +0,7] (3) | sans puissance |
| Interwetten | 8 % | +12,14 % (120) | — (0) | sans puissance |
| VC Bet | 5 % | +3,47 % (213) | — (0) | sans puissance |
| Ladbrokes | 2 % | −0,42 % (52) | +1,39 % [−1,2 ; +4,0] (7) | sans puissance |
| 1xBet | 3 % | +3,57 % (142) | — (0) | sans puissance |
| **Dixon-Coles** | 0 % | −7,71 % (1 648) | **−7,57 % [−7,9 ; −7,2] (1 962)** | **rejetée** |

**Le test est vide pour les règles « prix mou ».** Après janvier 2026, football-data n'a plus Pinnacle et la plupart des
bookmakers ; un seuil à 3 % ne retient d'ailleurs que 1 à 2 % des matchs sur l'histoire (0,3 % au test pour Bet365).
Aucun verdict n'est donc possible :
ni validation, ni infirmation. Le protocole n'a pas été modifié après coup pour obtenir un verdict.

**Stabilité (Bet365, seuil 3 %, toute l'histoire).** CLV par saison : +0,3 ; +3,4 ; +0,7 ; +0,1 ; −0,7 ; −2,9 ; +9,6 ;
−1,0 ; −4,0 ; −0,5 (2016/17 à 2025/26 ; 305 paris sur toute l'histoire). Négatif dans 5 saisons sur 10, dont les trois dernières
complètes : rien qui ressemble à un avantage stable. L'intervalle du CLV d'entraînement (+1,09 % [−0,3 ; +2,5]) contient 0.

**Espérance annoncée contre rendement réalisé.** Pour Bet365 au test, le rendement réalisé ne monte pas avec l'espérance
annoncée (tranche 2 à 5 % : −43,7 % sur 15 matchs ; tranche < −5 % : +13,6 % sur 194), signe que la règle ne distingue pas
les bons des mauvais prix à cette taille d'échantillon.

**Dixon-Coles : rejeté, et ruineux avec des mises Kelly.** Sur 1 962 paris du test : probabilité annoncée 32,5 %, fréquence
réelle 25,5 %, clôture du marché 26,9 % ; espérance annoncée +16,6 %, rendement réalisé −12,4 %. Plus l'espérance annoncée
est grande (≥ 10 % : 1 050 matchs), plus le CLV est mauvais (−8,5 %). Bankroll de 1 000 € sur le test :

| Kelly | Paris placés | Bankroll finale | Drawdown max | Ruine (< 10 %) si la clôture dit vrai |
|---|---|---|---|---|
| ×1 | 514 | 3 € | 100 % | 98 % |
| ×½ | 797 | 21 € | 98 % | 62 % |
| ×¼ | 1 203 | 121 € | 88 % | 2 % |

Avec la décote française (cotes × 0,957 pour Betclic, × 0,910 pour Winamax, provisoires), le CLV du modèle passe de
−7,6 % à −12,0 % puis −16,6 %.

**Valeur en direct (`edge value`, premier relevé, 72 matchs à venir dont 62 avec le 1X2 complet de Pinnacle).** Seuil
3 % : aucun prix. Seuil 0 % : trois matchs, tous chez PMU, sur le nul (espérances +1,2 %, +0,1 %, +2,4 %). Cohérent avec
la section 17 : les opérateurs français ne laissent presque aucune valeur contre Pinnacle.

**Défaut trouvé et corrigé par le rapport.** L'approximation du second ordre du Kelly simultané (plus de 12 paris dans un
lot) ne garantissait pas une somme de mises inférieure à 1 ; avec les avantages annoncés du modèle, elle dépassait
la bankroll et le drawdown simulé atteignait 103 %. La somme est désormais plafonnée à 95 % (commit `8058587`, test dédié).
Les trois exécutions du modèle déjà journalisées avant la correction (`backtest_run` 22 à 24) sont donc fausses ; elles ne
peuvent pas être réécrites (journal immuable) et attendent la décision de Paul. Le rapport recalcule tout et n'en dépend pas.

## 19. Phase 5 — vérifications (2026-10-18)

**Cotes d'un seul match (clôture).** Documentation (https://the-odds-api.com/liveapi/guides/v4/) : `GET /v4/sports/{sport}/events/{eventId}/odds`,
paramètres `apiKey`, `regions`, `markets`, `bookmakers` (prioritaire sur `regions`), `dateFormat`, `oddsFormat` ; « cost = [number of
unique markets returned] x [number of regions specified] » ; le décompte porte sur les marchés **renvoyés** ; les réponses vides ne
comptent pas ; `/events` (liste des matchs) ne coûte rien. **Mesuré** avec `markets=h2h&bookmakers=pinnacle` sur un match de
La Liga : HTTP 200, `x-requests-last: 1`, `x-requests-used: 15`. La réponse est **un objet** (pas une liste) de la même forme qu'un
élément de `/odds` (`id`, `sport_key`, `commence_time`, `home_team`, `away_team`, `bookmakers[].markets[].outcomes[]`). Elle est
enregistrée dans `crates/core/tests/fixtures/odds_event_pinnacle.json`. Pinnacle y annonçait une mise à jour datant de quatre minutes
au moment de l'appel : le retard de la « page publique » n'est pas mesuré mais ne dépasse pas quelques minutes.
Non vérifié : coût quand Pinnacle n'a aucune cote pour le match (réponse sans bookmaker : 0 ou 1 crédit ?) ; réponse pour un match
retiré ou terminé (le code journalise l'appel, ne rejoue qu'une fois, et signale si le coût réel diffère du coût prévu).

**Définition de la clôture en direct.** Un relevé de Pinnacle pris dans les 30 minutes avant le coup d'envoi. Avec un passage toutes
les dix minutes, il est pris entre 20 et 30 minutes avant le match, alors que la clôture de football-data est prise plus près du
coup d'envoi : le CLV en direct sous-estime légèrement les derniers mouvements de Pinnacle. À garder en tête en comparant au
CLV du backtest.

**Budget mensuel prévu.** Relevé quotidien : 6 crédits × 30 = 180. Clôtures : 1 crédit par match suivi (recommandations notifiées,
journal fantôme et paris réels). Sur le premier relevé, 3 matchs sur 62 avaient un avantage ≥ 0 % : de l'ordre de 3 à 10 matchs
suivis par semaine, soit 15 à 45 crédits par mois. Total attendu : 200 à 230 crédits sur un plafond de 450. La garde de budget
refuse tout appel qui le dépasserait.

**Webhook Discord** (https://docs.discord.com/developers/resources/webhook, anciennement discord.com/developers/docs/…, redirection 301).
« Execute Webhook » : `POST /webhooks/{id}/{token}`, sans authentification autre que le jeton de l'URL ; `content` jusqu'à
2 000 caractères ; `allowed_mentions` pour interdire les mentions ; fichiers en `multipart/form-data` avec `payload_json` et
`files[n]` ; réponse 204 par défaut (`wait=false`). Le programme n'envoie jamais de mention (`allowed_mentions.parse = []`),
n'accepte qu'une URL commençant par `https://discord.com/api/webhooks/` (ou `discordapp.com`), refuse les redirections et
expurge l'URL et le jeton de toute erreur. Non vérifié : taille maximale d'une pièce jointe et limites de débit (absentes de la
section lue) ; l'envoi réel n'a pas été essayé, faute de webhook.

## 20. Phase 6 — vérifications (2026-10-18)

**Versions (crates.io, API publique du 2026-10-18).** `eframe` : 0.37.0 (15/10/2026, Rust 1.95), 0.36.2 (08/09/2026, Rust 1.95),
0.35.0 (25/06/2026, Rust 1.92). `egui_plot` : 0.37.0 (05/08/2026) dépend de `egui ^0.36`, 0.36.0 de `egui ^0.35`, 0.35.0 de
`egui ^0.34`. **Les numéros ne sont pas synchronisés** : eframe 0.37 (egui 0.37) n'a pas encore d'`egui_plot` compatible. Paire
retenue, validée par Paul : `eframe 0.36.2` + `egui_plot 0.37.0` (un seul `egui 0.36.2` dans l'arbre de dépendances, vérifié par
`cargo tree -i egui`). Fonctionnalités d'eframe : `wgpu`, `wayland`, `x11`, `default_fonts` ; ni `glow` ni `accesskit`.

**Dérive de l'API d'egui entre les versions que je connaissais et la 0.36.** `Context::run` devient `Context::run_ui` (la
fermeture reçoit un `Ui`), le trait `eframe::App` n'a plus `update(ctx, frame)` mais `ui(ui, frame)`, les panneaux s'écrivent
`Panel::top("id").show(ui, …)`, `ComboBox::from_id_salt`. Un `FullOutput` doit être consommé (`drop_without_applying_deltas`)
dans un test sans fenêtre, sinon `epaint` panique. Tout a été lu dans les sources de la 0.36.2 (`~/.cargo/registry`).

**Machine de Paul.** Session Wayland (`WAYLAND_DISPLAY=wayland-1`), Vulkan disponible : Intel UHD 630 (Mesa) et NVIDIA GTX 1650.
Lancement de `edge-gui` pendant 8 secondes sur les vraies données : aucune erreur sur la sortie d'erreur. Première compilation
en release : environ 3 minutes ; en développement : environ 1 minute 20.

**Ce qui n'est pas vérifié.** Aucune capture d'écran n'a été faite (je ne capture pas l'écran de Paul : il contient d'autres
fenêtres). Le rendu visuel (largeur des colonnes, lisibilité sur petit écran) n'est testé que par des tests qui dessinent chaque
onglet sans fenêtre et n'échouent pas.

## 21. Phase 7 — vérifications avant le tennis (2026-10-18)

**tennis-data.co.uk** (page http://tennis-data.co.uk/data.php, `notes.txt`, `robots.txt`).
- Fichiers **uniquement tableur** : ATP 2000 en `.xls` sans cotes, 2001–2012 en `.xls`, 2013–2026 en `.xlsx` ; WTA 2007–2012 en `.xls`,
  2013–2026 en `.xlsx`. Aucun CSV ni zip. Les chemins de téléchargement portent un segment opaque (`hrjk-85Hyt…/{année}/{année}.xlsx`).
- Colonnes de résultats : tournoi, lieu, date, série (ATP) ou tier (WTA), terrain (intérieur/extérieur), **surface**, tour, nombre
  de sets maximal, vainqueur, perdant, classements et points ATP/WTA au début du tournoi, jeux par set, sets gagnés, commentaire
  (terminé, abandon, forfait). Avant 2003, la date de tous les matchs d'un tournoi est celle du début du tournoi.
- Cotes : une colonne gagnant (`…W`) et une perdant (`…L`) par bookmaker (B365, B&W, CB, EX, LB, GB, IW, **PS** = Pinnacle, SB, SJ,
  UB), plus `Max` et `Avg` d'Oddsportal. **Une seule cote par bookmaker : « generally the most recent before play starts »**, soit
  quasiment la clôture. Il n'existe pas de cote d'avant-match distincte : le CLV historique est impossible en tennis, et
  décider sur ces cotes serait décider sur la clôture (invariant 4).
- **Conditions d'usage** : « Tennis-Data maintains full copyright over the data files in their spreadsheet format. Its use is intended
  for private individuals only, not commerical or data training products using automated bots/scrapers/AI. » Le `robots.txt` autorise
  tout robot sauf `/stuff/`, mais interdit nommément les robots d'entraînement d'IA (GPTBot, Google-Extended, **Anthropic-AI,
  Claude-Web, ClaudeBot**, PerplexityBot, CCBot…). Usage personnel : permis. Téléchargement automatisé par un programme : zone grise,
  d'où une décision de Paul. Je n'ai pas téléchargé les fichiers de données moi-même.
- Lecture des tableurs en Rust : `calamine 0.36.1` (27/07/2026, licence MIT, Rust 1.88, lit `.xls` et `.xlsx`).

**Jeff Sackmann (`tennis_atp`, `tennis_wta`).** Les dépôts `JeffSackmann/tennis_atp` et `JeffSackmann/tennis_wta` répondent **404**
(API GitHub et `raw.githubusercontent.com`) ; la liste des dépôts publics de son compte ne contient plus que
`tennis_MatchChartingProject` (dernière activité 18/09/2026). Je ne sais pas s'ils ont été supprimés ou rendus privés. Un miroir tiers
existe (Hugging Face `Aneeshers/tennis-sackmann-archive`, licence CC BY-NC-SA 4.0 reprise de l'original, dernière modification
25/06/2026 : ATP et WTA « jusqu'en 2026 », mais figé à cette date). La licence autorise l'usage non commercial avec attribution ;
le retrait des dépôts d'origine est un signal à prendre en compte, et le miroir ne se met plus à jour.

**The Odds API et le tennis.** Une clé de compétition **par tournoi** (`tennis_wta_ningbo_open`…), présente seulement pendant le
tournoi : le 18/10/2026, `GET /sports` (gratuit) liste 1 clé de tennis sur 77. La liste fixe de ligues du relevé de football ne convient
donc pas ; il faudrait découvrir les clés actives. Mesuré sur ce tournoi (1 crédit, `markets=h2h` et 6 bookmakers demandés) : 12 matchs,
issues à deux (pas de nul), Pinnacle sur 9 matchs (marge moyenne 3,5 %), PMU sur 11 (5,4 %), Unibet sur 12 (10,4 %), Winamax sur 12
(9,4 %), **Betclic absent**. Les marges tennis des opérateurs français sont nettement plus basses que celles du football (Winamax
14,2 % au premier relevé de football).

**Conséquence sur le plan.** Les cotes historiques de tennis-data sont des quasi-clôtures : on peut comparer un modèle au marché
(log loss, Brier, calibration, test d'information), mais pas mesurer un CLV ni simuler des décisions d'avant-match. Seule l'archive en
direct, qui sépare prix d'avant-match et clôture, permettra de juger un avantage en tennis.

**Elo de tennis (Tennis Abstract, « An Introduction to Tennis Elo », J. Sackmann, 2019-12-03).** Formule : `1 − 1/(1 + 10^(écart/400))`
(écart de 256 points → environ 81,4 % pour le mieux classé). Départ « traditionnel » à 1500, jugé « complètement arbitraire ».
Quatre classements par joueur (général, dur, terre, gazon ; pas de séparation intérieur/extérieur), prévision par un **mélange 50/50**
du classement de surface et du classement général (« my testing showed that 50/50 worked for each surface »). L'auteur n'a
« yet to find a tweak to the system that consistently improves its ability to predict match outcomes », sauf une pénalité d'absence.
Aucune valeur numérique de `K` n'est donnée. Les prévisions de l'Elo y sont « pas tout à fait aussi bonnes que les cotes ».
La page de FiveThirtyEight sur son Elo de tennis (2016) redirige désormais vers ABC News : **je n'ai donc pas pu vérifier la forme
`K = 250/(n+5)^0,4`** que je croyais en venir. Le code utilise `K(n) = k_base/(n+5)^k_shape` comme **choix du projet** (réglé sur la
validation, 45 configurations), sans source à citer. Autres choix propres au projet : le poids de surface est réglé (la source fixe 0,5) ;
un classement de surface naît égal au classement général du joueur (la source ne le précise pas).

**Archive du tennis en direct.** Le relevé quotidien découvre les tournois en cours (`GET /v4/sports`, gratuit, clés `tennis_…` actives
sans marché « vainqueur du tournoi ») et archive chacun pour 1 crédit (6 au plus par relevé, réglable). Estimation : environ 2 tournois
par jour en moyenne, soit 60 crédits par mois, pour un total attendu de 255 à 285 sur le plafond de 450 (football 180, clôtures 15 à 45).
Le football passe avant le tennis : si le budget manque, le tennis est refusé. Les lectures de recommandation, de clôture et de
saisie hors modèle sont restreintes au football (clé `soccer_…`) : le 1X2 suppose un nul.

## 22. Reste à vérifier

- Date de réinitialisation du quota gratuit : le compte date du 2026-10-18, donc probablement le 18 de chaque mois
  (non vérifié). La garde ne l'utilise pas ; `edge snapshot` signale toute baisse de `x-requests-used`.
- Colonnes Pinnacle de football-data.co.uk après juillet 2025 (phase 2).
- Comportement réel de 11 bookmakers ou plus (2 crédits ?) et d'une réponse vide (0 crédit ?).
- Plage de Pinnacle entre 201 h et 313 h avant le coup d'envoi.
- Fréquence de mise à jour de Pinnacle (délai « public website ») : mesurable avec l'archive.
- Liste officielle des codes d'erreur et comportement exact du 429.
- Phase 2 : origine et définition des colonnes `HxG` / `AxG` de football-data (2026/27) ; raison du retrait de
  Pinnacle ; conditions d'usage de l'API FPL ; couverture réelle d'API-Football (Écosse, compositions, blessures) ;
  retour de ClubElo.
- Marge de Pinnacle selon l'horizon (heures avant le coup d'envoi) dans notre archive, contre les 2,6 à 2,9 % de
  football-data.
- Conditions d'usage de football-data.co.uk (aucune mention trouvée) et origine des `HxG` / `AxG`.
- Phase 3 : le meilleur réglage est en bordure de grille (ξ maximal, κ minimal) ; une grille étendue n'a pas été
  essayée pour ne pas multiplier les sessions de réglage.
- Cotes de clôture en direct et décote française réestimée sur l'archive (phase 5) : seule façon de tester les règles
  « prix mou » sur Winamax et Betclic, faute d'historique gratuit.
- Exécutions `backtest_run` 22 à 24 (modèle) calculées avant le plafond du Kelly simultané : à purger ou à ignorer.
- Phase 5 : coût d'un appel par match sans cote de Pinnacle ; réponse pour un match retiré ou terminé ; taille maximale
  d'une pièce jointe Discord et limites de débit ; première notification réelle (attend le webhook de Paul).
- Retard réel des cotes de Pinnacle dans The Odds API (champ `last_update` contre l'heure de l'appel), à mesurer sur l'archive.
- Phase 6 : rendu visuel réel de l'application (Paul), comportement sous X11, passage à eframe 0.37 quand un `egui_plot`
  compatible sera publié.
- Phase 7 : statut exact des dépôts de Sackmann (supprimés ou privés), colonne de dates et noms de joueurs de tennis-data (homonymes),
  couverture de Pinnacle en tennis (9 matchs sur 12 sur le seul tournoi mesuré), tournois couverts par The Odds API sur une année.
- Phase 7 : la forme de `K` n'a pas de source vérifiée ; coût réel du relevé de tennis sur une semaine de Grand Chelem (nombre de
  clés actives) ; conversion meilleur des 3 / meilleur des 5 (non faite).
