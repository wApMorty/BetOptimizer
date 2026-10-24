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

## 22. Phase 7 — mesures sur les fichiers réels (`edge tennis …`, 2026-10-18)

**Import.** 28 tableurs (ATP et WTA, 2013 à 2026) : 68 251 matchs écrits, 15 lignes rejetées (ATP 2022, le Masters Cup de novembre
n'a pas de colonne « Best of ») ; de 0 à 28 paires de cotes écartées par fichier (cote ≤ 1 ou une seule des deux).
**Pinnacle (`PS`)** est présent sur 98 à 99 % des matchs de 2013 à 2024, 95 % en 2025, mais seulement **3 à 4 % en 2026** : comme
football-data, tennis-data a perdu Pinnacle cette année. Le test (à partir de 2024) est donc, pour la comparaison au marché,
essentiellement 2024 et 2025.

**Contrôle qualité.** 40 paires de cotes aberrantes (marge hors de [1,00 ; 1,30], surtout la moyenne `Avg` et quelques Pinnacle de
juillet 2019) : signalées, écartées de la comparaison, jamais corrigées. **Dates approximatives** : 1 244 couples (joueur, date) où un
joueur apparaît deux fois le même jour dans le même tournoi (demi-finale et finale datées du même jour après une pluie) ; l'Elo
rejoue donc par vague (date, tour du tournoi). 14 paires de tours sur 19 159 ont le tour avancé daté avant le tour précédent.
**Homonymes** : un seul nom apparaît à la même date dans deux tournois (WTA, Babos T., 2018-01-04). Le premier détecteur (même
date, tout tournoi confondu) donnait 430 noms, tous des dates approximatives : il a été corrigé avant tout réglage.

**Réglage de l'Elo (une seule session, 45 configurations, validation 2022-2023, 8 420 matchs).** Retenu : `K = 150/(n+5)^0,4`, poids de
surface 0,25, log loss 0,6266. Cinq meilleures : (150 ; 0,4 ; 0,25) 0,6266, (150 ; 0,4 ; 0,5) 0,6272, (150 ; 0,3 ; 0,25) 0,6280,
(150 ; 0,3 ; 0,5) 0,6281, (250 ; 0,5 ; 0,25) 0,6282. **Le meilleur `k_base` est le plus petit de la grille** (150) : un `K` plus petit
n'a pas été essayé, pour ne pas multiplier les sessions. Le mélange des surfaces compte peu (0,25 contre 0,5 : 0,0006).

**Test (à partir du 2024-01-01).** 14 923 matchs, dont 12 624 terminés avec deux joueurs de 10 matchs d'expérience au moins, dont 8 620
avec une cote Pinnacle plausible (comparaison au marché, 1 cote aberrante écartée).

| | Log loss | Brier |
|---|---|---|
| Elo par surface (gelé) | 0,6240 | 0,2178 |
| Pinnacle sans marge | 0,5947 | 0,2052 |
| Écart Elo − marché | **+0,0293 [0,0242 ; 0,0344]** | +0,0126 [0,0104 ; 0,0147] |

Par tranche (écart de log loss, positif = l'Elo est moins bon) : dur +0,0266, terre +0,0340, gazon +0,0329 ; ATP +0,0340, WTA +0,0244 ;
meilleur des 3 +0,0271, **meilleur des 5 +0,0512** [0,0333 ; 0,0686] (809 matchs : l'Elo ignore le format, où le favori gagne plus
souvent). Toutes les tranches ont un intervalle entièrement au-dessus de zéro. **Test d'information : le poids optimal de l'Elo dans le
mélange avec le marché est 0** (choisi sur la validation) : l'Elo ne contient rien que Pinnacle ignore. Même conclusion qu'en football.

## 23. Phase 8 — vérifications avant la NBA (2026-10-18)

**The Odds API et la NBA.** `GET /v4/sports` (gratuit) : `basketball_nba` actif (46 matchs à venir, premier coup d'envoi le 2026-10-20 à
19 h UTC), plus `basketball_nba_championship_winner` (marché « vainqueur », écarté). Marchés : `h2h`, `spreads` (handicaps), `totals`
(plus/moins) ; « mainly available for US sports and bookmakers ». Coût = marchés × régions : mesuré **3 crédits** pour 3 marchés et 6
bookmakers (`x-requests-last: 3`). Résultats : `GET /v4/sports/basketball_nba/scores/?daysFrom=1..3` coûte **2 crédits**
(1 sans `daysFrom`, mais alors seulement les matchs en cours et à venir) ; réponse : `id`, `commence_time`, `completed`, `home_team`,
`away_team`, `scores`, `last_update`. Historique : 10 crédits par région et par marché, **plans payants seulement**.

**Couverture mesurée (46 matchs, relevé du 2026-10-18).** Pinnacle : `h2h`, `spreads` et `totals` sur 14 matchs (ceux des jours
proches), marges 3,1 %, 3,0 % et 3,5 %. Opérateurs français : **`h2h` seulement** chez Winamax (23 matchs, marge 10,5 %), Betclic
(23, **7,1 %**), Unibet (23, 10,0 %), PMU (21, 10,0 %) et NetBet (12, 9,5 %) ; handicaps seulement chez NetBet (marge 18,5 %), totaux
seulement chez NetBet (18,2 %) et PMU (16,5 %). Les opérateurs français ne sont donc comparables à Pinnacle que sur le vainqueur.

**Données historiques gratuites : aucune source vérifiée ne convient.** FiveThirtyEight `nbaallelo.csv` (via datahub.io) : aucune licence
indiquée (« All rights reserved »), résultats issus de Basketball-Reference, couverture non précisée, FiveThirtyEight a fermé ses
pages. `shufinskiy/nba_data` (Apache-2.0) : play-by-play de stats.nba.com jusqu'à 2024/25, sans conditions d'usage de la NBA ni cotes.
Jeu « NBA Games » de Nathan Lauga (Kaggle, copie à jour au 2025-02-13) : licence inconnue. Base d'écarts et de totaux de Princeton
(2006-2023) : inaccessible à un programme (page protégée). Basketball-Reference : page « bot traffic » en HTTP 403. Cotes historiques
de l'API : payantes. Conclusion : **archive propre**, comme le prévoit CLAUDE.md ; la première comparaison modèle-marché ne sera
possible qu'après quelques centaines de matchs archivés (début décembre 2026).

**Elo avec marge de victoire.** Les pages méthodologiques de FiveThirtyEight (NBA, WNBA, calcul de l'Elo) redirigent toutes vers ABC News.
Par recoupement avec un résumé de recherche, non vérifié à la source : multiplicateur de marge `(marge + 3)^0,8 / (7,5 + 0,006 × écart d'Elo)`
(écart négatif si l'outsider gagne), avantage du terrain de 100 points d'Elo constant, `K` de 28 pour la WNBA (la valeur NBA n'est
pas confirmée). Ces valeurs servent de point de départ ; elles seront réglées sur les données et présentées comme des choix du projet.

**Budget et calendrier retenus (décisions de Paul, 2026-10-19).** Relevé NBA dédié à 22 h UTC (une unité systemd, même coût qu'un relevé de
14 h), trois marchés (3 crédits par jour), résultats tous les deux jours (2 crédits, `daysFrom=3`, recouvrement d'un jour), tennis refusé
avant la NBA (`nba_reserve_credits` = 120 retirés de son plafond). Estimation mensuelle : football 180, tennis environ 60, NBA 90 + 30,
clôtures 15 à 45, soit 375 à 405 sur 450. Premier relevé réel (2026-10-19) : 46 matchs, 378 cotes, 5 crédits. Fenêtres du modèle fixées dans
le code avant les données : validation du 2026-11-15 au 2027-02-01, test ensuite ; `edge nba tune` est refusé avant le 2027-02-01.

## 24. Phase 9 — refonte de l'interface (2026-10-19)

**Police.** Inter 4.0 (Rasmus Andersson), https://github.com/rsms/inter, tag `v4.0`, fichiers `Inter-Regular.ttf` et
`Inter-SemiBold.ttf` (environ 410 Ko chacun) dans `assets/fonts/` (à la racine du dépôt depuis la phase 10, partagée avec l'image Discord). Licence SIL Open Font License 1.1 (texte `OFL.txt`
joint) : l'incorporation dans un programme est autorisée, les fichiers ne sont pas modifiés. Aucune nouvelle dépendance Cargo.

**Principes de design.** Ils viennent de connaissances générales et de lectures secondaires faites pendant la session, **sans
vérification à la source** : durées de mouvement d'interface de l'ordre de 150 à 300 ms, décalage d'apparition en cascade, surfaces
sombres teintées plutôt que noir pur, une seule couleur d'accent. Les valeurs chiffrées attribuées à Material Design 3 n'ont pas été
relues dans sa documentation. Toutes les valeurs de l'interface (palette, espacements 4-8-12-16-24-32, rayons 14 et 9, durées de 0,28 s,
0,7 s et 0,9 s) sont des **choix du projet**, réglés à l'œil sur des captures d'écran.

**Contraste.** Seule mesure vérifiable : le rapport de contraste WCAG 2.x (formule de la luminance relative ci-dessus, dans
`design/tokens.rs`), exigé à 4,5 :1 par un test pour chaque couleur de texte et chaque surface, dans les deux thèmes. Le seuil de 4,5 :1
pour le texte courant (niveau AA) est une valeur que je cite de mémoire ; elle n'a pas été relue dans le texte de la norme.

**egui 0.36.2 / egui_plot 0.37.0.** API vérifiée dans les sources des crates installées : `ViewportCommand::Screenshot` et
`Event::Screenshot`, `Ui::new_child` avec `UiBuilder`, `Context::globally_used_rect`, `Plot::grid_color` / `grid_spacing`,
`FontDefinitions`. Un `horizontal_wrapped` ne passe à la ligne que devant un composant simple (comportement constaté par le test de
non-débordement, pas documenté).

## 25. Pinnacle après janvier 2026 : pistes et mesure de Betfair Exchange (2026-10-19)

**Constat dans notre base.** Pinnacle (`PS`) : 19 109 cotes d'avant-match de 2016-08-06 au **2026-01-15**, puis plus rien (957 matchs sur 1 980 en
2025/26, 0 en 2026/27). Pinnacle a fermé son API publique en juillet 2025 (source secondaire : odds-api.io, « pinnacle-api-shutdown-alternatives ») ;
je n'ai pas trouvé d'annonce de football-data.co.uk sur l'arrêt de ses colonnes `PS*` (la page `notes.txt` répond HTTP 429).

**Sources de cotes historiques Pinnacle trouvées (rien n'a été téléchargé).**
- Kaggle « Football Odds Movements 2018-2026 » (archangel0x01, 63 597 matchs, juillet 2018 à juin 2026, Pinnacle ouverture et clôture) : **licence et
  provenance non vérifiables** (la page ne se charge pas pour un programme). Même règle que pour le jeu Kaggle de la NBA (§23) : sans licence lue,
  on ne l'utilise pas ; Paul peut la lire lui-même. Les cotes viennent probablement d'un relevé automatique du site de Pinnacle, dont les conditions
  ne sont pas vérifiées.
- TipsAudit (tipsaudit.com/data), **CC-BY-4.0** : 5 202 pronostics de juin à octobre 2026 avec cotes d'ouverture et de clôture de Pinnacle, sans clé ni
  inscription. Inutilisable comme base de backtest : ce sont **leurs** pronostics (sélection biaisée), pas tous les matchs.
- Offres payantes (The Odds API historique à 10 fois le coût, Apify, API de cotes commerciales) : hors budget (CLAUDE.md : gratuit uniquement).
- Notre propre archive en direct : Pinnacle depuis le 2026-10-18 (ligne de départ de la seule source licite et gratuite).

**Mesure : Betfair Exchange (`BFE`) remplace-t-il Pinnacle comme référence ?** Football-data publie encore `BFE*` (exchange Betfair) et `Avg*`
jusqu'à aujourd'hui. Sur les **2 936 matchs** où `PS` et `BFE` existent (2024/25 et 2025/26 jusqu'au 2026-01-15), retrait de marge par la méthode
puissance, `scripts/bfe_vs_pinnacle.py` (lecture seule) : écart absolu moyen des probabilités 0,63 point (médiane 0,50) à l'avant-match, 0,58 à la
clôture ; log loss PS 0,9640 et BFE 0,9640 à l'avant-match (Δ +0,0001 [−0,0024 ; +0,0025]) ; à la clôture PS 0,9628, BFE 0,9619 (Δ −0,0009
[−0,0019 ; +0,0001]) ; `Avg` fait de même (0,9638 et 0,9618). Surronde moyenne : PS 1,037, BFE 1,013 (maximum 2,35 : quelques lignes aberrantes
à écarter). Intervalle : approximation normale sur les matchs pris comme indépendants (pas de bootstrap par blocs). Conclusion mesurée : sur ces
matchs, BFE est **indiscernable de Pinnacle** comme référence sans marge. Non établi : que cela tienne hors des grandes ligues, ni que BFE soit
aussi « tranchant » que Pinnacle pour repérer un prix mou.

**Diagnostic sur la validation seule (2024/25, 1 980 matchs, Pinnacle présent partout ; `scripts/soft_refs_validation.py`, lecture seule).** Prix mou
dépassant la référence d'au moins 3 % (meilleure issue du match, marge retirée par la méthode puissance), CLV contre la clôture de Pinnacle :
Bet365 contre Pinnacle **12 paris**, CLV −4,0 % (± 7,9) ; contre Betfair Exchange (lignes avec surronde ≤ 1,10) **3 paris**, −30,6 % ; contre la moyenne du
marché 4 paris, −3,0 % ; Bwin contre Pinnacle 2 paris (+2,8 %), contre Betfair 3 (−20,1 %). Sur 2019/20 à 2023/24 : Bet365 contre Pinnacle 103 paris
(+0,85 % ± 2,2), contre la moyenne 40 (−1,15 %). Le nombre de paris de Bet365 au seuil de 3 % contre Pinnacle baisse de saison en saison dans le rapport
de backtest (59 en 2016/17, 36 en 2019/20, 12 en 2024/25). **Conclusion mesurée : l'écart « prix mou contre Pinnacle » s'est tari sur ces relevés ;
l'absence de Pinnacle après janvier 2026 n'est pas la cause principale du faible nombre de paris au test, et Betfair n'en ajoute pas.** Aucune session de
réglage supplémentaire n'a été lancée : Paul avait dit « go » sur le plan, mais le diagnostic montre qu'elle n'apporterait rien, et le choix lui a été rendu.
Limite : les cotes « avant-match » de football-data sont relevées plusieurs jours avant le match (§17 : 137 h en moyenne), pas au moment où un parieur les verrait.

## 26. Scores exacts : disponibilité des cotes et ordre de grandeur de la variance (2026-10-19)

**Documentation** (the-odds-api.com/sports-odds-data/betting-markets.html) : pour le football, les marchés `correct_score` (« Correct Score at Regulation Time »)
et `correct_score_h1` existent ; ils ne se lisent pas dans `/odds` mais match par match (`/events/{id}/odds`), au coût de 1 crédit par marché et par
région. Les bookmakers qui les proposent ne sont pas listés par la page.

**Sonde réelle (`scripts/probe_correct_score.py`, 1 crédit, un seul match : Tottenham – Coventry, région `eu`).** Trois bookmakers seulement : Pinnacle
(30 scores, somme des 1/cote = 1,214), William Hill (46 scores, 1,526), MyBookie (25 scores, 1,401). **Aucun opérateur français** n'a répondu pour ce
match (non généralisable : un seul match sondé). La marge d'un score exact est donc de l'ordre de 20 % chez Pinnacle (sur les 30 scores listés) et de
40 à 50 % ailleurs, contre 3 à 4 % pour le 1X2 de Pinnacle.

**Historique : aucun.** football-data.co.uk ne publie pas de cotes de scores exacts (colonnes de l'en-tête `E0_2526.csv`) ; The Odds API historique est payant.
Seule une archive en direct permettrait de juger, à raison d'au moins 1 crédit par match et par relevé : tout relevé de tous les matchs du jour (environ
100) dépasserait de loin le plafond gratuit de 500 crédits par mois (déjà environ 360 estimés).

**Variance d'un pari de score exact sans aucun avantage** (calcul, approximation normale) : probabilité 0,11, cote 7,58 (cote juste 9,09 moins 20 % de marge),
espérance −16,7 %, écart-type d'un pari 2,37 mises. Écart-type du rendement après 100 paris : 23,7 points (probabilité de finir positif sans avantage : 24 %) ;
300 paris : 13,7 points (11 %) ; 1 000 paris : 7,5 points (1,3 %). Quelques centaines de paris de scores exacts ne distinguent donc pas un avantage de la chance.

## 27. Abaque des seuils et opérateurs suivis (`edge backtest curve`, 2026-10-20)

**ParionsSport.** Revérifié le 2026-10-20 sur the-odds-api.com/sports-odds-data/bookmaker-apis.html : ni `parionssport_fr`, ni FDJ, ni ZEbet
dans les régions `fr` (5 clés : `betclic_fr`, `netbet_fr`, `pmu_fr`, `unibet_fr`, `winamax_fr`) et `eu`. Aucun remplacement possible.
**NetBet** : écarté à la demande de Paul (`excluded_bookmakers`). Ses cotes restent archivées (aucun crédit de plus jusqu'à dix bookmakers),
il n'est plus jamais recommandé ni montré.

**Protocole.** Pour chaque bookmaker mou de football-data, seuils de 0 à 8 % par pas de 0,1 point (81 seuils × 8 sources = 648 configurations),
entraînement 2016/17 à 2023/24 et validation 2024/25 seulement. Mesures : paris retenus, CLV moyen (bootstrap par blocs ligue × semaine) et
croissance logarithmique totale attendue `G = Σ_lots E_clôture[ln W]`, mises de Kelly simultané sur Pinnacle d'avant-match, issues tirées selon
la clôture de Pinnacle sans marge (méthode `power`). Calcul en 22 s sur une copie de la base. **Rien n'est gelé** à cette date.

| Source | Paris à 0 % | CLV à 0 % | Seuil de G maximal (×1) | G ×1 au maximum | G ×½ à 1 % | G ×¼ à 1 % |
|---|---|---|---|---|---|---|
| Bet365 | 1 772 | +0,05 % [−0,4 ; +0,5] | 7,0 % (56 paris) | **−0,012** (négative à tous les seuils) | +0,013 | +0,019 |
| William Hill | 1 330 | +0,14 % | 1,3 % | −0,009 | +0,036 | +0,030 |
| Bet&Win | 1 292 | +0,32 % | 1,2 % | +0,038 | +0,039 | +0,025 |
| Interwetten | 3 029 | **+2,29 % [+2,0 ; +2,6]** | 0,7 % | **+1,66** (bankroll × 5,2 en 9 saisons) | +1,22 | +0,71 |
| VC Bet | 1 672 | −0,61 % | 5,6 % | −0,055 | −0,033 | −0,005 |
| Ladbrokes | 184 | −0,73 % | 1,6 % | +0,006 | +0,002 | +0,002 |
| Betfred | 0 | — | — | — | — | — |
| 1xBet | 1 114 | **+1,69 % [+1,2 ; +2,1]** | 0,5 % | **+0,146** (une saison, × 1,16) | +0,101 | +0,061 |

**Kelly complet et avantages surestimés.** Chez Bet365, le CLV moyen est positif à presque tous les seuils (+0,88 % à 3 %), mais la croissance en
Kelly complet est négative partout : l'avantage annoncé (≥ 3 % à ce seuil) vaut environ trois fois l'avantage réel mesuré par la clôture, et miser au-delà
du double du Kelly optimal fait décroître la bankroll même quand l'espérance est positive. Demi et quart de Kelly restent positifs mais minuscules
(bankroll × 1,01 à × 1,02 en neuf saisons). Chez Interwetten et 1xBet, l'avantage est réel et la croissance décroît quand le seuil monte : les petits
avantages rapportent.

**Le nul.** Chez 1xBet, le CLV du nul vaut +3,2 % à 0 % (447 paris) contre +1,0 % pour le domicile et 0,0 % pour l'extérieur ; chez Bet365, +0,6 %
contre +0,0 % et −0,4 %. Les quatre lignes « fantômes » de l'archive réelle (Winamax ×2, Unibet ×2) sont toutes des nuls.

**Précision de l'avantage.** Pour une même sélection, l'avantage diffère selon la méthode de retrait de marge de 0,50 à 0,68 point en médiane et de
1,8 à 5,3 points au 90e centile selon le bookmaker. Un pas de 0,1 point est donc plus fin que la mesure : le seuil est un ordre de grandeur.

**Gel (2026-10-20, deuxième session, 648 configurations, critère : croissance attendue en demi-Kelly).** Paul a choisi le demi-Kelly
après avoir vu que Kelly complet perdait chez Bet365 à tous les seuils. Seuils gelés : Bet365 7,0 % (plateau : 0,016 contre 0,013 à 1 %,
indiscernables), William Hill 0,6 %, Bet&Win 0,1 %, Interwetten 0,7 %, VC Bet 5,6 %, Ladbrokes 1,6 %, 1xBet 0,5 %. **Test, évalué une seule
fois** : Bet&Win 147 paris, CLV −0,55 % [−1,5 ; +0,2] (indécis) ; Ladbrokes 14 paris ; toutes les autres sources 0 pari (Interwetten et 1xBet
ont disparu des fichiers, Bet365 n'atteint pas 7 %). Le test ne départage donc rien : le seuil des opérateurs français ne peut pas s'appuyer
sur un verdict historique.

**Production (décision de Paul, 2026-10-20, démarche exploratoire).** Seuil des recommandations notifiées : 3 % → **1 %** ; fraction de Kelly par
défaut : 1,0 → **0,5**. Aucun crédit de plus : tout avantage positif était déjà journalisé en fantôme avec sa clôture.

**Mises minimales des opérateurs (vérifié le 2026-10-20).**
- **Winamax** : « Le montant minimum pouvant être parié est de 0,10€ » ; cote minimale 1,05 (winamax.fr/cgu-paris-sportifs, article « Cote et
  mise minimale », règlement officiel en ligne).
- **Betclic** : le règlement des paris sportifs du 15/05/2023 (copie publiée par un tiers, bonus-malin.info) fixe une cote minimale de 1,10
  (§ 1.3.1) mais **aucune mise minimale** pour le sport ; les 0,50 € qu'il cite portent sur l'hippique. Les comparateurs donnent 0,10 €
  (certains 1 €, page ancienne qui cite encore l'ARJEL). Règlement officiel actuel non lisible sans navigation : à confirmer dans le panier.
- **PMU** : règlement du 05/07/2022 (pmu.fr), Titre V, art. 2.2.2 : minimum et maximum « fixés exclusivement par le PMU, sont indiqués au
  parieur dans son panier », différents selon les sports et compétitions ; mise au centime près. Les comparateurs donnent 0,50 €.
- **Unibet** : 0,10 € selon les comparateurs ; règlement officiel non consulté.
Conséquence : la mise minimale de la configuration (`min_stake` = 1 €) est plus stricte que celle de tous les opérateurs ; c'est elle qui
écarte les petites mises, pas les opérateurs.

## 28. Site de rapports (`edge serve`, 2026-10-20)

Demande de Paul : le rapport HTML joint à Discord s'ouvre mal (il arrive comme un fichier de code). Remplacé par un site en lecture seule
sur le réseau local, et un simple rappel de l'URL dans le message.

**Serveur HTTP : `tiny_http` 0.12.0** (vérifié par `cargo info` et dans les sources du paquet, 2026-10-20). Licence MIT ou Apache-2.0 ;
fonctionnalités par défaut vides (TLS seulement sur option : non activé) ; dépendances `ascii`, `chunked_transfer`, `httpdate`, `log`.
Synchrone : les connexions sont acceptées par un thread et un petit pool internes, `Server::incoming_requests` nous remet les requêtes
une à une (aucun async, conformément à CLAUDE.md). `Request::respond` n'envoie pas le corps à une requête `HEAD`
(`do_not_send_body = self.method == Method::Head`, `src/request.rs`). Dernière version publiée en 2022 : bibliothèque stable, peu active.
Alternative écartée : un `TcpListener` écrit à la main, qui obligerait à analyser HTTP nous-mêmes.

**Choix.**
- Le routage et les pages sont dans `core` (`report/site.rs`, testables sans réseau) ; la CLI ne fait que la traduction HTTP.
- Base ouverte en `SQLITE_OPEN_READ_ONLY` à chaque requête ; méthodes autres que `GET`/`HEAD` refusées (405).
- Un rapport archivé n'est servi que si son nom est exactement celui d'un fichier listé par l'archive (format `AAAA-MM-JJ-HHMMSSZ.html`) :
  aucun chemin construit à partir de l'URL.
- En-têtes : `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; img-src data:` (aucun script, aucune ressource
  externe), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cache-Control: no-store`.
- Paper trading : seules les recommandations notifiées **avec une mise** (et un avantage positif) comptent, choix de Paul : « je ne parie
  pas » les autres. Mises en unités de la bankroll du jour de la recommandation.
- Pare-feu du portable : `ufw` actif, `avahi-daemon` actif (nom `cachyos-x8664.local`), constaté le 2026-10-20. Le port n'est pas ouvert
  par l'outil : c'est à Paul de le faire.

**Image des recommandations dans Discord : `resvg` 0.48.1** (validé par Paul le 2026-10-20 ; vérifié par `cargo info` et dans les sources).
Licence MIT ou Apache-2.0, Rust pur. Seule la fonctionnalité `text` est activée (`default-features = false`) : ni polices du système
(`system-fonts`), ni images matricielles, ni SVG compressé. Version minimale de Rust : 1.85 pour `resvg`, `usvg`, `harfrust`, `skrifa`,
`read-fonts` (1.71 pour `fontdb`), compatible avec les 1.89 du cœur. `tiny-skia` 0.12 encode le PNG avec ses fonctionnalités par défaut
(`png-format`). API utilisée : `usvg::Options { font_family, fontdb, .. }`, `usvg::Tree::from_str`, `resvg::render`, `Pixmap::encode_png`.
Police : Inter (SIL OFL 1.1), déjà embarquée par l'application, déplacée dans `assets/fonts/` à la racine pour être partagée ; environ
820 Ko de plus dans le binaire `edge`. PNG plutôt que JPG : texte net, et Discord l'affiche directement sous le message. L'image n'est
jointe que s'il y a au moins une recommandation ; son échec n'empêche pas l'envoi du message.

## 29. Cotes boostées : historique de WePari et étude de rentabilité (`edge boost study`, 2026-10-20)

**Source.** wepari.fr, site personnel non rattaché aux opérateurs. Mentions légales (wepari.fr/mentions.html, lues le 2026-10-20) :
éditeur particulier, données « à titre indicatif », certaines cotes boostées possiblement absentes ; **aucune clause** sur la propriété
intellectuelle ou la réutilisation ; pas de `robots.txt` (404). Le site annonce que les opérateurs ont rendu l'accès automatique aux cotes
boostées plus difficile depuis le premier trimestre 2025, et que ses relevés n'ont pas été adaptés. Avec l'accord de Paul, deux pages
téléchargées **une seule fois** le 2026-10-20 (`curl`, une requête chacune), conservées dans `~/.local/share/edge/wepari/` et jamais
retéléchargées :
- `indexh.html` (11,4 Mo) : historique complet de **Winamax**, 26 984 boosts du 29/08/2021 au 08/06/2025, tous sports. Colonnes : pari,
  cote boostée, cote non boostée, boost %, fin du pari (heure de Paris), événement, gain net. Issue codée par la couleur de la cellule de
  la cote (vert gagné, orange perdu, gris non réglé : 1 327 lignes, surtout depuis mars 2025). Pas de colonne de mise maximale, mais
  **règle donnée par Paul le 2026-10-20** : 10 € pour une « Grosse Cote Boostée », 50 € pour les autres. Les gains nets de WePari
  (mise = −gain si perdu, gain ÷ (cote − 1) si gagné) la confirment : 1 149 « grosses » sur 1 152 à 10 €, 24 486 autres sur 24 506 à 50 €.
- `indexpsel.html` (2,2 Mo) : historique « récent » de **Parions Sport**, 4 695 boosts du 21/12/2022 au 10/08/2025, avec la **mise
  maximale** (25 € : 2 393 ; 50 € : 1 493 ; 10 € : 795 ; autres : 15).
Betclic et Unibet figurent sur la page d'accueil (Coupe du monde 2026), sans historique exploitable.

**Méthode.**
1. Rapprochement : couples d'équipes tirés de l'intitulé de l'événement (formes de Winamax et de Parions Sport), résolus par la table
   d'alias (55 variantes ajoutées : « FC Barcelone », « Naples », « Mayence »…) contre les matchs du même jour en base. Règle du projet
   conservée : un nom inconnu dans un pari évaluable **bloque** l'étude.
2. Traduction de l'intitulé en conditions sur le score final (victoire, double chance, les deux équipes marquent, plus/moins de buts,
   écart, victoire sans encaisser, scores exacts, buts d'une équipe, et leurs conjonctions). Tout le reste (mi-temps, buteurs, premier
   but…) est déclaré non évaluable. Contrôle : l'issue déduite du score en base concorde avec celle de WePari pour **1 523 boosts sur
   1 526 (99,8 %)**.
3. Prix juste : grille de Dixon-Coles **calée sur le marché** (λ, μ, ρ ajustés pour reproduire victoire domicile, victoire extérieur et
   plus de 2,5 buts de Pinnacle sans marge, méthode `power`), puis somme des scores gagnants. Pinnacle d'abord, moyenne du marché à défaut.
   Deux moments : avant-match (pour filtrer, sans fuite du futur) et clôture (pour mesurer).
4. Choix de la grille sur 2019/20–2020/21, **avant** la première donnée de WePari : ρ ajusté (log loss moyenne de 14 paris de référence
   0,49814), ρ = 0 (0,49856), ρ = −0,1 (0,49818). **Trois configurations essayées**, ρ ajusté retenu.
5. Intervalles à 95 % par bootstrap par blocs d'une semaine (2 000 tirages, graine 20261020).

**Calibration de la grille retenue (clôture de Pinnacle, 8 951 matchs de 2021/22 à 2025/26).** Bonne sur les totaux de buts et les
combinés « victoire + les deux équipes marquent » (|z| < 1). **Biais nets** : les deux équipes marquent annoncé 51,6 %, observé 54,0 %
(z = +4,6) ; victoire à domicile par 2 buts d'écart ou plus 24,7 % contre 22,6 % (z = −4,9) ; double chance + les deux marquent 36,9 %
contre 39,0 % (z = +4,2) ; l'extérieur marque au moins 2 buts 34,1 % contre 35,5 % (z = +3,0). Conséquence : sur ces familles, l'avantage
calculé d'un boost porte une erreur de modèle de l'ordre de 2 points de probabilité, soit environ 4 points d'espérance à une cote de 2.

**Gain réel, tous sports, sans modèle** (tous les boosts réglés par WePari, mise fixe, bootstrap par blocs d'un mois) :

| Opérateur | Mise maximale | Boosts réglés | ROI réalisé [IC 95 %] |
|---|---|---|---|
| Winamax | 10 € (« Grosse Cote Boostée ») | 1 147 | **+25,3 % [+19,1 ; +31,7]** |
| Winamax | 50 € | 24 510 | **−4,3 % [−6,1 ; −2,5]** |
| Parions Sport | 10 € | 777 | +3,4 % [−4,4 ; +10,8] |
| Parions Sport | 25 € | 1 673 | −4,9 % [−9,6 ; +0,3] |
| Parions Sport | 50 € | 1 432 | −5,8 % [−15,3 ; +4,0] |

C'est le résultat le plus solide de l'étude, parce qu'il ne dépend ni de la grille, ni du rapprochement, ni de la traduction des
intitulés. Jouer **toutes** les Grosses Cotes Boostées de Winamax à 10 € aurait rapporté environ 25 % des mises, soit environ 27 boosts
par mois (2021-2025) et de l'ordre de **65 € par mois**. Les boosts ordinaires perdent en moyenne, et aucune règle ne les rend
sûrement gagnants.

**Résultats, Winamax (Big 5 + Écosse).** 3 385 boosts rapprochés (12,5 % des boosts publiés), 1 403 évaluables (les autres dépendent
surtout de la mi-temps ou d'un buteur).
- Ensemble : espérance **+1,3 % [−0,1 ; +2,7]** contre une marge d'origine de −14,8 % ; ROI réalisé +3,4 % [−5,5 ; +12,5]. En moyenne, le
  boost ramène la cote au prix juste, pas au-delà.
- Familles à éviter (intervalle entièrement négatif) : victoire + les deux marquent −4,8 % (237), score exact −5,5 % (200), victoire sans
  encaisser −3,4 % (134), double chance + les deux marquent −4,0 % (103), pas de nul + les deux marquent −4,6 % (39).
- Les deux équipes marquent seul : +23,9 % [+20,1 ; +27,7] annoncé, mais ROI réalisé −9,3 % et calibration z = −2,5 sur ces 74 boosts,
  alors que la grille **sous-estime** ce marché sur l'ensemble des matchs : signe probable d'**antisélection** (l'opérateur booste ce pari là
  où il sait que la probabilité est plus faible que ne le dit un modèle tiré du 1X2 et des totaux). À ne pas croire sans le marché
  « les deux équipes marquent » de Pinnacle.
- Par cote boostée : 2,00 à 2,50, espérance +9,9 % [+7,5 ; +12,2] et ROI réalisé **+18,4 % [+5,8 ; +29,9]** (337 boosts), seul groupe dont
  le gain réel exclut zéro ; à prendre comme hypothèse (une vingtaine de groupes regardés : comparaisons multiples).
- Règle sans fuite du futur, avantage d'avant-match > 0 : **494 boosts, 11,7 par mois**, espérance à la clôture +15,8 % [+12,9 ; +18,7],
  ROI réalisé **+9,0 % [−4,6 ; +21,9]**. Seuil plus haut : espérance annoncée plus forte (+33,5 % au-delà de 10 %) mais ROI réalisé
  inchangé (+12,6 % [−3,4 ; +28,9]) : l'espérance annoncée croît plus vite que le gain réel, autre signe d'erreur de modèle.
- **Par mise maximale, le résultat le plus net de l'étude.** Grosses Cotes Boostées (10 €) : 186 boosts, espérance **+28,5 %
  [+25,5 ; +31,5]**, 98 % positives, marge d'origine −6,8 %, ROI réalisé +15,4 % [−0,8 ; +32,6]. Autres boosts (50 €) : 1 217 boosts,
  espérance **−2,9 % [−4,1 ; −1,7]**, marge d'origine −16,0 %, ROI réalisé +1,6 % [−8,0 ; +11,9]. Avec la règle « avant-match > 0 » :
  - « grosses » : 183 retenues sur 186, 4,3 par mois, espérance +29,6 % [+27,2 ; +32,4], ROI réalisé **+17,3 % [+0,6 ; +35,5]** (seul
    groupe de la règle dont le gain réel exclut zéro, de justesse) ; gain attendu à la mise maximale **environ 13 € par mois** (7 € au ROI
    réalisé) ;
  - autres : 311 retenues, 7,4 par mois, espérance +7,6 % [+4,1 ; +11,7], du même ordre que l'erreur de la grille ; ROI réalisé +4,1 %
    [−13,1 ; +22,7] ; gain attendu à la mise maximale environ 28 € par mois **si** la grille est juste.

**Résultats, Parions Sport.** 990 boosts rapprochés, 183 évaluables : espérance **−2,8 % [−4,6 ; −0,8]**, marge d'origine −17,8 %. Règle
« avant-match > 0 » : 54 boosts (1,9 par mois), +7,4 % annoncés, gain attendu à la mise maximale **environ 1,4 € par mois** (les boosts
les plus avantageux sont les « CB FLASH » plafonnés à 10 €). Sans intérêt économique.

**Limites.** Le relevé d'avant-match de football-data date du vendredi ou du mardi, pas de l'heure de publication du boost. La règle des
mises maximales de Winamax est celle de 2026 ; les gains de WePari montrent qu'elle valait déjà de 2021 à 2025. WePari peut avoir omis des
boosts. Les familles dépendant de la mi-temps (environ 400 boosts Winamax)
ne sont pas tarifées. Aucun seuil n'est gelé : étude exploratoire.

## 30. Paris par thème et clubs « réguliers » (étude ponctuelle, 2026-10-20)

**Question de Paul.** Parier un thème (tous les matchs d'un club, tous les matchs à domicile, à l'extérieur, les nuls) en
cherchant un taux de réussite élevé, les pertes étant censées être couvertes par la gestion des mises. Exemples cités : tous
les matchs des Rangers sauf contre le Celtic, tous les matchs du Celtic à domicile.

**Rappel de principe.** Espérance d'un pari : `p·o − 1` ; fraction de Kelly `f* = (p·o − 1)/(o − 1)` (Kelly, 1956), nulle ou
négative dès que l'espérance l'est. Aucune gestion des mises ne rend positive une espérance négative ; une progression
(martingale) déplace les pertes et ajoute un risque de ruine. Le taux de réussite dépend de la cote, pas de la qualité du pari.

**Méthode.** Scripts Python ponctuels, en lecture seule sur la base, non versionnés (Paul n'a pas voulu de commande). Cotes
Pinnacle d'avant-match de football-data (19 109 matchs, 2016/17 à janvier 2026 ; 2025/26 partielle, 2026/27 absente), mise
fixe. Deux prix : Pinnacle, et Winamax obtenu par la décote mesurée ci-dessous.

**Décote française par tranche de cote (archive The Odds API).** Rapport médian cote française / cote Pinnacle, même issue et
même relevé ; 22 relevés du 2026-10-18 au 2026-10-20, 146 matchs : observations corrélées (un match revient à chaque relevé).

| Cote Pinnacle | < 1,3 | 1,3–1,6 | 1,6–2,2 | 2,2–3,5 | 3,5–6 | ≥ 6 |
|---|---|---|---|---|---|---|
| Winamax (n) | 0,944 (15) | 0,958 (63) | 0,944 (155) | 0,913 (259) | 0,912 (317) | 0,807 (59) |
| Betclic (n) | 0,952 (13) | 0,962 (60) | 0,953 (151) | 0,940 (257) | 0,934 (314) | 0,869 (59) |

La décote n'est pas uniforme : elle est faible sur les favoris (≈ 0,95) et forte sur les outsiders. Les facteurs uniques
provisoires du §18 (× 0,910 Winamax, × 0,957 Betclic) pénalisent donc trop les favoris de Winamax et pas assez ses outsiders.
Première mesure seulement, à refaire quand l'archive aura quelques mois.

**Thèmes généraux** (décote uniforme du §18) :

| Thème | Paris | Réussite | ROI Pinnacle (± 1σ) | Betclic | Winamax |
|---|---|---|---|---|---|
| Tout à domicile | 19 109 | 44,0 % | −4,0 % (1,0) | −8,1 % | −12,7 % |
| Tout à l'extérieur | 19 109 | 31,1 % | −6,3 % (1,3) | −10,4 % | −14,8 % |
| Tous les nuls | 19 109 | 24,9 % | −2,5 % (1,3) | −6,7 % | −11,2 % |
| Favori | 19 109 | 54,3 % | −1,0 % (0,7) | −5,3 % | −9,9 % |
| Favori < 1,50 | 3 966 | 76,4 % | −0,2 % (0,9) | −4,5 % | −9,2 % |
| Favori < 1,25 | 1 181 | 83,1 % | −2,5 % (1,3) | −6,7 % | −11,3 % |
| Outsider ≥ 6 | 5 226 | 9,4 % | −15,1 % (4,0) | −18,7 % | −22,7 % |

**Clubs, Winamax à la décote par tranche :**

| Thème | Paris | Réussite | Réussite nécessaire chez Winamax | ROI Pinnacle | ROI Winamax |
|---|---|---|---|---|---|
| Rangers, tous sauf Celtic | 306 | 69,3 % | 72,7 % | −3,1 % | −8,1 % |
| Celtic à domicile sauf Rangers | 149 | 83,9 % | 88,3 % | 0,0 % | −5,5 % |
| Celtic à domicile | 166 | 81,3 % | 83,6 % | +0,7 % | −4,9 % |

Une seule saison du Celtic à domicile (2017/18 : 53 % de réussite, ROI −36 %) efface cinq saisons positives.

**Recherche automatique des clubs réguliers.** 307 combinaisons club × thème (tous, domicile, extérieur, « sauf Old Firm » pour
les deux clubs de Glasgow), présentes au moins 6 saisons avec au moins 8 paris par saison.
- Aux cotes Pinnacle, 19 combinaisons ont au moins 75 % de saisons positives (Fribourg à domicile 9/10, Newcastle à domicile 8/9,
  Aston Villa 6/7…), 118 un ROI global positif. Avec la décote Winamax : 4 et 39.
- **Hasard** : 100 historiques simulés, issues tirées selon les probabilités sans marge de Pinnacle d'avant-match (méthode
  multiplicative, graine 1). On obtient en moyenne 10,9 combinaisons aussi régulières (maximum 20) et 115 à ROI positif. L'excédent
  observé (19 contre 11) est compatible avec le biais favori/outsider du §14, que la méthode multiplicative ne reproduit pas ; il
  n'est pas propre à un club.
- **Walk-forward** (le test décisif) : chaque saison, jouer toutes les combinaisons positives sur chacune des trois saisons
  précédentes. 2019/20 à 2025/26, 3 789 paris : **ROI −4,8 % aux cotes Pinnacle, −12,1 % chez Winamax**, positif chez Winamax une
  saison sur sept (+0,1 %). Corrélation du ROI d'un club d'une saison à la suivante : 0,063 sur 792 paires (non significative).
  Les 10 clubs les plus rentables d'une saison (+18 à +58 %) font −2,5 % en moyenne la saison suivante.

**Conclusion.** Aucun thème ni aucun club n'est régulièrement rentable ; le marché intègre déjà la force des clubs, et un thème
choisi sur son passé ne prédit rien. Seule piste qui recoupe une mesure indépendante : le nul chez certains opérateurs (§27, CLV
du nul +3,2 % chez 1xBet ; lignes fantômes de l'archive toutes sur le nul), déjà couverte par le moteur de valeur. Décision de Paul
le 2026-10-20 : aucune commande, résultat seulement documenté. Environ 320 configurations regardées en tout.

## 31. Bonus de bienvenue couverts (2026-10-20 et 2026-10-21, phase 12)

**Pistes écartées avant de retenir celle-ci** (scripts ponctuels, lecture seule, non versionnés).
- **Approche « Moneyball » : prédire le mouvement de la cote.** Régression linéaire du mouvement Pinnacle vendredi → clôture sur
  plusieurs variables : écart entre les bookmakers moins bien informés et Pinnacle, colonne `Max`, marge, ligue. Entraînement jusqu'à
  2023/24, test 2024/25 à janvier 2026 (1 235 matchs). R² hors échantillon 0,002 (domicile) et −0,003 (extérieur) ; log loss de la
  clôture prédite −0,0005 ± 0,0004 contre Pinnacle du vendredi. CLV d'une sélection chez les bookmakers moins bien informés :
  +0,0 % à +1,5 % ± 4, indiscernable de zéro. Quatre configurations regardées sur le test. Avec le §16 (poids optimal du modèle : 0),
  conclusion : rien à attendre d'un apprentissage sur ces données publiques.
- **Arbitrage entre opérateurs français** (archive, 2026-10-18 au 2026-10-20, 919 lignes 1X2 de football). Aucun : la meilleure
  cote française sur chaque issue garde au moins 5,7 % de marge (9,3 % en moyenne). 13 lignes sur 919 ont de la valeur contre Pinnacle,
  dont 10 sur le nul (Unibet, PMU).

**Règle d'éligibilité vérifiée.** Règlement Winamax de l'offre de bienvenue du 18 avril 2025, clause (C) (PDF lu le 2026-10-21) : offre
valable « une seule fois par utilisateur, terminal […], foyer, carte bancaire […] ou compte bancaire », « réservée aux joueurs n'ayant jamais
effectué de dépôt sur un compte Winamax (y compris sur un compte qui a été fermé depuis) ». Paul a touché des offres une seconde fois
après une pause de 2019 à 2022 : règles différentes à l'époque, autres opérateurs, ou offres de retour. Non tranché ; chaque règlement
est à lire avant tout dépôt (`edge bonus offer --source …`).

**Opérateurs agréés** (anj.fr, liste lue le 2026-10-20) : seize sociétés, dont bet365.fr (présent en France depuis mai 2026, selon les
comparateurs). Parions Sport et ZEbet n'y figuraient pas : à revérifier. Seuls Winamax, Betclic, Unibet, PMU et NetBet ont des cotes
dans The Odds API (région `fr`, §27) ; pour les autres, le calcul se fait sur les cotes affichées.

**Mesures de couverture sur l'archive** (3 jours, meilleur match de chaque jour ; détail dans `docs/bonus-bienvenue.md` §3 à §6) :
- freebet ou remboursement en argent, couvert : 0,53 à 0,56 par euro, à une cote de 3,4 à 5 ;
- premier pari remboursé en freebet, entièrement couvert : 0,17 à 0,22 par euro, à une cote de 2,5 à 3,7 ;
- une grosse cote coûte plus qu'elle ne rapporte : avec la couverture, le gain baisse et l'argent à sortir monte (Betclic, 100 € :
  22 € garantis et 243 € sortis à 2–3, contre 7 € et 668 € à 6,5–10).

Une première estimation (« un premier pari couvert est à peu près nul ») était fausse. Elle reposait sur une marge moyenne et non sur le
meilleur match du jour ; elle a été corrigée avant le choix du mode.

**Premier essai réel de `edge bonus status`** (copie de la base, relevé du 2026-10-20 à 12:00 UTC, couverture chez Winamax seul) :
- PMU, remboursé en argent de 100 € : +53,60 € garantis pour 281 € sortis ;
- Betclic, remboursé en freebet de 100 € : +14,50 € garantis.

Une fois PMU consommé et devenu compte de couverture : Betclic +15,47 €.

**Choix de Paul.** Tout couvrir (2026-10-21) : il veut être sûr de retrouver l'argent avancé. Site en lecture seule, sans saisie ; les
bonus restent hors de la courbe de bankroll du modèle.

**Règlements lus le 2026-10-22** (pages officielles ; citations courtes) :
- **PMU** (pmu.fr/ouverture-compte/conditions/conditions-offre.html, offre Sport du 01/10/2024 au 31/12/2026) : réservée à qui ouvre un
  compte « pour la première fois, ou plus de 2 ans après la clôture de son dernier compte » ; 1er pari dont le résultat est connu sous
  15 jours ; s'il est perdant, l'enjeu est « recrédité sous forme de Bonus cash […] dans la limite de 100€ » (25 % sous 10 jours ouvrés,
  75 % après confirmation du compte) ; exclus : paris système, Ticket Sport, paris annulés ou remboursés, « remboursé si égalité »,
  CashOut. Statut retirable du « Bonus cash » : non trouvé (la page des conditions générales est rendue en JavaScript, illisible ici).
- **Betclic** (betclic.fr/content/spo_wbt, sans date) : « nouveaux joueurs Betclic n'ayant pas eu de compte ouvert dans les 12 derniers
  mois » ; « jusqu'à 100€ remboursés en Feebets si ton premier pari sportif est perdant », crédits « non retirables » ; seuls les paris
  en cash simples et combinés comptent. Cote minimale, durée de validité : non trouvées (CGU non lues).
- **Unibet** (unibet.fr/paris-sportifs/bonus, offre du 01/09 au 26/10/2026) : « 100% du montant de votre 1er pari seront crédités […]
  sous forme de bonus bet non retirables et dans la limite de 100€ », crédit « après le résultat de votre 1er pari », **sans condition
  de perte** ; 1er dépôt et 1er pari de 5 € minimum dans les 60 jours ; CashOut exclu ; 365 jours pour utiliser les bonus bet. Exclusion
  commune aux trois bonus : prise de jeu en argent réel chez Parions Sport, Unibet, ZEbet ou ZEturf dans les 24 derniers mois (le joueur
  ou son foyer), ou offre de bienvenue Unibet déjà reçue. Règlement des paris sportifs (applicable au 29/09/2026), art. 3.3 : BonusBet gagnant,
  « Gain net = mise x (cote – 1) » ; plusieurs BonusBet cumulables dans une prise de jeu. Aucune clause sur les paris couverts dans ce
  règlement.
- Conséquence pour l'outil : l'offre Unibet n'est aucune des trois formes de `edge bonus` (le bonus est dû même si le 1er pari gagne).

## 32. Reste à vérifier

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
- Phase 7 : réglage de l'Elo en bordure de grille (`k_base` minimal) ; conversion meilleur des 3 → meilleur des 5, dont l'absence
  explique une partie de l'écart sur les Grands Chelems masculins ; couverture de Pinnacle par tennis-data après 2025.
- Phase 8 : formule exacte de l'Elo NBA de FiveThirtyEight (K, repos, retour à la moyenne) ; part des matchs NBA avec des lignes
  Pinnacle à 24 h et à 2 h du coup d'envoi ; date de début des lignes de handicap et de total chez Pinnacle.
- Phase 8 : lignes de handicap et de total de Pinnacle sur les matchs lointains (14 matchs sur 46 le 2026-10-19) ; effet du passage à l'heure
  d'hiver américaine (17 h au lieu de 18 h à New York) sur la proximité des cotes avec la clôture ; traitement des séries éliminatoires.
- Phase 9 : seuil AA de 4,5 :1 et formule de luminance à relire dans la norme WCAG ; durées de mouvement et valeurs de Material Design 3 ;
  rendu des animations à l'œil sur une session réelle (Paul) ; comportement sous X11 et sur écran à forte densité de pixels.
- Phase 9 (suite) : licence et provenance du jeu Kaggle « Football Odds Movements 2018-2026 » ; annonce de football-data sur l'arrêt des colonnes Pinnacle ;
  commission et horodatage exacts des colonnes `BFE` (non documentés dans ce que j'ai pu lire).
- Site de rapports : résolution de `cachyos-x8664.local` depuis les autres appareils de Paul (Windows résout mDNS depuis la version 10,
  pas toujours) ; comportement de `tiny_http` face à un client lent qui garde la connexion ouverte (une seule requête traitée à la fois).
- Scores exacts : couverture de `correct_score` par bookmaker et par ligue (un seul match sondé) ; présence ou non des opérateurs français dans l'API.
- Cotes boostées : paiement du gain boosté en argent réel ou en paris gratuits
  selon l'opérateur et l'époque ; marché « les deux équipes marquent » de Pinnacle dans The Odds API (clé de marché, coût, couverture) pour
  tarifer sans la grille ; heure de publication des boosts.
- Phase 12 : règlement de l'offre de bienvenue de chaque opérateur (seul celui de Winamax a été lu) ; valeur réelle de conversion
  d'un freebet couvert (`bonus_freebet_value`, 0,53 supposé) ; règle de chaque opérateur pour un match annulé pendant un premier pari
  remboursé ; présence de Parions Sport et ZEbet dans la liste de l'ANJ.

## 33. Recherche d'un avantage restant : latence, combinés boostés, promus, littérature (2026-10-21)

Demande de Paul : « tous les moyens » pour améliorer le modèle et trouver un système rentable. Scripts ponctuels en lecture seule
(scratchpad, non versionnés) ; trois recherches documentaires. Aucun crédit dépensé.

**Archive en direct** (2026-10-18 au 2026-10-21, 384 couples relevé × match avec le 1X2 de Pinnacle, 4 317 lignes françaises, méthode
`power`).
- Espérance moyenne contre Pinnacle sans marge : de −9 % à −16 % selon l'opérateur et l'issue (Winamax, extérieur : −16,2 %). Surronde
  moyenne 1,107 (PMU) à 1,132 (Winamax).
- Meilleure issue française de chaque match : médiane −5,5 %, 90e centile −2,3 %, 99e centile +0,7 %. Le nul est la meilleure issue
  dans 22 % des matchs.
- **Suivi des mouvements de Pinnacle** (2 901 paires de relevés successifs espacés d'au moins 12 h) : pente du mouvement français sur
  celui de Pinnacle 0,70. Quand Pinnacle bouge de 3 points de probabilité (162 cas), les opérateurs français bougent de 2,1 à 2,4 points :
  l'espérance passe de −12,2 % à −9,0 %, toujours négative. Le retard existe à l'échelle du jour, il ne couvre pas la marge.
- Relevés à moins de 6 h du coup d'envoi : 60 lignes seulement (relevés de clôture), aucune au-dessus de −2 %. Trop peu pour conclure sur
  la fenêtre des compositions.

**Bonus permanents sur les combinés** (règlements lus le 2026-10-21 : Betclic Multiboost, cash, 5 % à 5 sélections jusqu'à 100 % à 23 ;
Winamax Combo Booster, tirage d'espérance 2,24 × base, en bonus à rejouer ; Unibet Combi Boosté, Bonus Bets, 50 % à 15 ; PMU CombiMax,
barème lu chez un tiers, 75 % à 15). Gain d'une mise 1 : `P · (1 + (O − 1)(1 + b)) − 1`, `O` = produit des cotes, `P` = produit des
probabilités sans marge de Pinnacle (matchs distincts supposés indépendants). Avec les n meilleures sélections de chaque relevé :
espérance de −11 % (PMU, 4 sélections) à −63 % (Betclic, 20). Marge réelle par sélection : 3,6 % à 7,3 %, contre un point mort de 0,7 %
à 3,6 % par sélection selon le barème. **Écarté.** Non mesuré : autres sports et autres marchés.

**Promus en début de saison** (Deutscher, Frick & Ötting 2018 ; football-data, Big 5 + Écosse, 2017/18 à 2026/27, cotes Pinnacle
d'avant-match). Pour ou contre le promu, journées 1–6, 7–12, 13+ : aucun rendement positif distinguable de zéro, CLV de −2 % à −8 %.
**Écarté.**

**Calibration du nul selon la méthode de retrait de marge** (Pinnacle avant-match, 19 109 matchs). Fréquence observée 0,2491 ;
multiplicative 0,2470, Shin 0,2451, additive 0,2444, `power` 0,2437 (−0,54 point, environ 1,7 erreur-type). `power` garde le meilleur
log loss global (0,96049 contre 0,96071) mais sous-estime le nul : la valeur repérée sur les nuls n'est pas un artefact de la méthode,
elle serait plutôt sous-évaluée.

**Littérature** (détail des références dans le compte rendu du 2026-10-21) :
- seul résultat mesuré contre la clôture de Pinnacle sur un grand échantillon : la méthode « Wisdom of the Crowd » de Buchdahl
  (football-data.co.uk, blog), 26 960 paris, valeur ≥ 2 % contre Pinnacle, rendement +4,9 %, valeur contre la clôture +2,9 %, un tiers des
  paris perdant leur valeur avant la clôture ;
- Kaunitz, Zhong & Kreiner 2017 (arXiv 1710.02824) : +3,5 % à +9,9 % selon le moment du relevé, contre la cote maximale, comptes limités
  ensuite ;
- aucun modèle sur données publiques (xG, notes de joueurs, tirs) n'est mesuré contre la clôture de Pinnacle ; les résultats positifs
  publiés le sont contre des cotes moyennes ou maximales, souvent sur une seule ligue, non répliqués ;
- latence des bookmakers : 1xBet suit Pinnacle en 6 à 12 min, bet365 en 47 à 67 min (blog d'OddsPapi, 28 matchs, indicatif) ; aucune
  étude sur la fenêtre des compositions.

**Sources de cotes à haute fréquence** (pages lues le 2026-10-21).
- **OddsPapi** : offre gratuite de 250 requêtes par mois ; Pinnacle, Betfair Exchange, Winamax, Unibet, PMU (pas Betclic) ;
  `/historical-odds` jusqu'à 3 bookmakers par appel, données depuis janvier 2026. Conditions d'usage introuvables (404) : à lire avant tout
  usage. Seul moyen gratuit trouvé de mesurer la latence française autour des compositions sans attendre l'archive.
- The Odds API payante : 20 000 crédits pour 30 $ par mois (hors budget « gratuit uniquement »).
- Betfair Exchange : API bloquée pour les adresses IP françaises, compte impossible pour un résident (échanges non agréés par l'ANJ).
- odds-api.io (nouvelles clés gratuites suspendues), SportsGameOdds, SportMonks, BetsAPI, Sportradar : Pinnacle ou cotes absents du gratuit.

## 34. Historique OddsPapi : latence française, marchés dérivés, tests issus du trading (2026-10-22)

**Source.** OddsPapi v4, offre gratuite (compte de Paul, clé dans `~/.config/edge/oddspapi.key`, 0600, hors dépôt). Conditions
(oddspapi.io/us/legal/terms, lues le 2026-10-21) : revente, redistribution, partage de clé et scraping interdits ; un usage
d'analyse personnel n'est pas exclu. Quota : 250 requêtes par mois ; **`/v4/historical-odds` n'est pas décompté** (documentation
« Requests & Quota », vérifié : compteur inchangé après deux appels), 5 s entre deux appels, 3 bookmakers au plus, et un seul
bookmaker avec un seul `outcomeId` pour `betfair-ex` et `polymarket`. Inclus : Pinnacle, Winamax, Unibet, PMU, ZEbet, bet365.fr,
bwin.fr, PokerStars, Genybet, Feelingbet, NetBet, Polymarket, Kalshi, Betfair Exchange ; **pas Betclic**. Marché 1X2 = 101
(issues 101/102/103), plus/moins 2,5 = 1010, BTTS = 104. Une entrée = un changement de prix. Malgré la documentation (« depuis
janvier 2026 »), l'historique de ces bookmakers commence en pratique début avril 2026 : 776 matchs exploitables (Big 5 + Écosse,
2026-04-10 au 2026-10-19). Requêtes consommées pour l'étude : 13.

**Protocole figé avant lecture** (scripts `scripts/oddspapi/` : `download.py` télécharge dans `hist/` du dossier courant, `latency.py` et `study.py` portent le protocole en tête, `trading_tests.py`, `progress.py` affiche l’avancement) : référence Pinnacle sans marge
(puissance) au même instant, clôture = dernier prix Pinnacle avant le coup d'envoi, issue de plus grand avantage, seuil 1 %,
fenêtre 60–15 min, un pari par match et par opérateur, succès si CLV ≥ +1 % et borne basse > 0 (bootstrap ligue × semaine).

**Résultat brut : « succès » sur les trois marchés, invalidé par un artefact de données.**

| Marché | Paris (fenêtre) | CLV brute |
|---|---|---|
| 1X2 | 102 (98 Winamax) | +5,1 % [+3,8 ; +6,4] |
| Plus/moins 2,5 | 18 | +4,2 % [+1,8 ; +6,8] |
| BTTS | 8 | +6,5 % [+2,6 ; +13,5] |

La « valeur » vient de cotes Winamax qu'OddsPapi ne relevait pas : en avril-mai, 0 changement relevé dans les 24 h avant le match
(cote figée depuis environ 300 h), 45 paris ; en août-septembre, 14 à 19 changements, 18 paris ; en octobre, 44 changements (relevé
complet, 98 % de cotes identiques à notre archive The Odds API sur 84 comparaisons), **0 pari**. Unibet, bien relevé sur toute la
période, ne donne que 4 paris.

**Données propres** (filtre de qualité ajouté après coup : au moins 10 changements relevés dans les 24 h) : 1X2, 24 paris sur 619
couples match × opérateur, CLV +3,0 % [+1,7 ; +4,7], σ = 4,5 %, cote médiane 3,5, tous entre le 2026-05-24 et le 2026-09-12 ; au moins
20 changements : 7 paris, +2,8 % [+0,4 ; +5,5]. Plus/moins : 1 pari ; BTTS : 0. Signal fragile : aucun pari en octobre, le mois le
mieux relevé ; le filtre n'exclut pas un reste d'artefact. Taille d'échantillon pour trancher (unilatéral 5 %, puissance 80 %) :
14 paris pour une CLV de 3 %, 32 pour 2 %, 127 pour 1 %.

**Tests issus du trading (descriptifs).**
- Momentum de Pinnacle (Moskowitz 2021, Sung et al. 2019) : la pente du mouvement jusqu'à la clôture sur le mouvement des 24 h
  précédentes vaut +0,33 [+0,06 ; +0,60] à 24 h ; elle est nulle à 6 h, 3 h et 60 min. R² de l'ordre de 1 % : inexploitable face
  à 10 % de marge.
- Durée d'une cote française au moins 1 % au-dessus de Pinnacle dans les 6 h avant le match (données propres) : Winamax 48
  épisodes, médiane 23,5 min (p25 2,7, p75 57) ; Unibet 49 épisodes, médiane 3,7 min (p25 1,0, p75 14). Une saisie à la main reste
  possible chez Winamax, rarement chez Unibet.

**Recherches documentaires du 2026-10-21** (détail dans le compte rendu de la session) :
- Polymarket : bloqué en France (décision ANJ du 16 juillet 2026, API comprises) ; écart avec Pinnacle de 0,5 à 1 point ; aucune
  preuve sérieuse qu'il mène Pinnacle. Lecture possible seulement via un agrégateur.
- Popularité et sentiment : en football européen, les bookmakers donnent de meilleures cotes aux clubs populaires (Forrest &
  Simmons 2008 ; Franck, Verbeek & Nüesch 2011) ; le signal Twitter de Brown et al. (2018) n'existe qu'en direct.
- Marchés dérivés (BTTS, double chance) contre une référence sharp : aucune étude ; testé ici, rien sur données propres.
- Buchdahl (football-data.co.uk, 2026-10-09, 61 979 paris) : part de l'avantage annoncé qui survit à la clôture, environ 30 % sous
  500 $ de limite Pinnacle, environ 75 % au-delà de 2 000 $.

**Mise en production (phase 13, 2026-10-22).** Le test prospectif est porté en Rust dans `edge` (le script Python de la veille est
retiré : un composant récurrent passe par `core`) :
- `edge latency fetch` : un passage par semaine (timer `edge-latency`, lundi 9 h) ;
- `edge latency report` : la règle et le verdict ;
- page `/latence` du site ;
- archive immuable : migration 14 (`oddspapi_call`, `oddspapi_fixture`, `oddspapi_price`).

Règle figée dans `crates/core/src/eval/latency.rs` (constantes) :
- celle de l'étude, plus le filtre de suivi de 10 changements en 24 h (20 en lecture secondaire) ;
- matchs à partir du 2026-10-22 ;
- verdict unique sur les 32 premiers paris ; échéance le 2027-01-18.

Faits relevés à la mise en route :
- `GET /v4/fixtures` répond **404 `FIXTURE_NOT_FOUND`** quand aucun match ne correspond, au lieu d'une liste vide : traité comme une liste
  vide ;
- un 4xx est décompté comme une requête (documentation « Requests & Quota ») ;
- consommation au 2026-10-22 : 33 requêtes sur 250.

Table d'alias : 80 variantes OddsPapi ajoutées (six ligues). Red Star, Rodez, Arbroath et Dunfermline (matchs de barrage) n'ont pas de
nom chez The Odds API : ils restent hors table et seraient signalés par le contrôle de qualité.

## 35. Modèles d'apprentissage automatique contre le marché : logistique, LightGBM, réseau de neurones (2026-10-21)

Demande de Paul : un modèle « plus puissant » (réseau de neurones) qui prédirait mieux les résultats que le marché, sur les mêmes
données publiques. Étude ponctuelle en Python (venv jetable : pandas 3.0.6, scikit-learn 1.9.1, LightGBM 4.7.0), scripts dans le
scratchpad, non versionnés ; lecture seule des CSV football-data. Aucun crédit dépensé.

**Protocole, validé avant tout calcul.**
- 20 513 matchs (six ligues, 2016/17 à 2026/27) ; 2016/17 sert au rodage des variables. Entraînement 2017/18 à 2023/24 (14 153 matchs),
  validation 2024/25 (1 980), **test unique** du 2025-07-01 au 2026-10-12 (2 325 matchs, les mêmes qu'au §16).
- 34 variables d'avant-match, calculées dans l'ordre chronologique (l'état d'une équipe est lu, puis mis à jour avec le résultat) :
  Elo (K = 20, avantage du terrain 65, multiplicateur d'écart de buts du World Football Elo), moyennes exponentielles (α = 0,08) des buts,
  tirs, tirs cadrés et corners pour et contre, forme aux points (α = 0,2), repos, matchs joués, ligue. **Test de fuite** : recalculées sur
  les seuls matchs antérieurs au 2025-01-01, les variables de ces matchs sont identiques au bit près.
- Variante A : variables seules. Variante B : plus les probabilités sans marge (`power`) de la moyenne d'avant-match, sa surronde et
  l'écart de logits.
- Trois modèles à hyperparamètres fixés a priori : régression logistique multinomiale (C = 1), LightGBM (feuilles 15, pas 0,02, arbres
  choisis par arrêt précoce sur la validation), MLP 32-16 (époques choisies sur la validation). Réajustement sur entraînement + validation
  avec le nombre d'itérations gelé, puis une seule prédiction du test.
- Juge 1 : log loss et RPS contre la moyenne du marché d'avant-match (Pinnacle absent du test), bootstrap par semaines civiles ; poids
  optimal du modèle dans le mélange géométrique p ∝ p_marché^(1−w) · p_modèle^w.
- Juge 2 : un pari par match sur l'issue de plus forte espérance si p · cote − 1 > seuil ; cote = moyenne d'avant-match × 0,977
  (Betclic = 0,957 × Pinnacle d'après le §18, Pinnacle = 1,021 × moyenne, médiane mesurée sur l'entraînement). Seuils 0, 2, 5, 8 % ;
  retenu sur la validation : CLV moyen le plus haut avec au moins 50 paris (0 % pour les six modèles).
- **Configurations essayées : 6 modèles et 24 couples modèle × seuil**, tous gelés avant la lecture du test.

**Validation (2024/25).** Marché d'avant-match 0,96205. Aucun modèle n'est meilleur : variante A de +0,0166 à +0,0223 ; variante B de
+0,0018 [−0,0016 ; +0,0051] (logistique) à +0,0056.

**Test.** Marché d'avant-match : log loss 0,97537, RPS 0,19702 ; clôture 0,97436. Parier toutes les issues au prix décoté donne un CLV
de −9,76 % (référence sans information).

| Modèle | Δ log loss [IC 95 %] | Δ RPS | Poids optimal [IC 95 %] | Paris | CLV [IC 95 %] | ROI [IC 95 %] |
|---|---|---|---|---|---|---|
| Logistique A | +0,0101 [+0,0046 ; +0,0156] | +0,0029 | +0,05 [−0,23 ; +0,29] | 1 462 | −9,4 % [−9,9 ; −8,9] | −11,6 % [−18,6 ; −4,2] |
| LightGBM A | +0,0112 [+0,0065 ; +0,0158] | +0,0033 | −0,01 [−0,23 ; +0,23] | 1 435 | −10,4 % [−10,9 ; −9,9] | −18,0 % [−24,6 ; −11,7] |
| MLP A | +0,0179 [+0,0103 ; +0,0249] | +0,0046 | +0,05 [−0,13 ; +0,22] | 1 862 | −8,7 % [−8,9 ; −8,4] | −7,7 % [−13,2 ; −1,9] |
| Logistique B | +0,0210 [+0,0132 ; +0,0293] | +0,0064 | +0,03 [−0,13 ; +0,21] | 2 110 | −9,9 % [−10,4 ; −9,5] | −10,9 % [−17,4 ; −3,9] |
| **LightGBM B** | **+0,0019 [−0,0015 ; +0,0053]** | +0,0003 | +0,25 [−0,26 ; +0,68] | 844 | −10,1 % [−10,7 ; −9,6] | −6,2 % [−16,2 ; +4,7] |
| MLP B | +0,0188 [+0,0080 ; +0,0295] | +0,0038 | +0,13 [−0,07 ; +0,33] | 1 946 | −8,3 % [−8,7 ; −7,9] | −7,2 % [−13,9 ; −0,3] |

Espérance annoncée des paris : +5,8 % (LightGBM B) à +15,6 % (MLP B) ; le CLV et le ROI la démentent, comme pour Dixon-Coles (§18).

**Verdict : piste close.** Aucun modèle ne bat le marché d'avant-match ; le meilleur (LightGBM avec les cotes en entrée) le rejoint au
mieux ; tous les poids optimaux sont compatibles avec 0. Plus de capacité ne compense pas l'absence d'information nouvelle : sur
20 000 matchs bruités, le réseau de neurones fait moins bien que la logistique.

**Limites.**
- Hyperparamètres fixés sans grille ; modèles gelés au 2025-07-01 (pas de réajustement hebdomadaire) ; pas de sortie de Dixon-Coles
  en variable.
- **Rupture de la surronde** de la moyenne d'avant-match : 1,046 à 1,053 de 2016/17 à 2024/25, 1,065 en 2025/26 et 1,066 en 2026/27
  (football-data a perdu Pinnacle et une partie des bookmakers). Les variantes B logistique et MLP extrapolent hors du domaine
  d'entraînement, d'où leur dégradation entre validation et test ; LightGBM y est insensible. Le verdict n'en dépend pas.
- Les choix des MLP (CLV −8,3 % et −8,7 %) sont environ 1,5 point meilleurs que la référence sans information (−9,8 %). Non vérifié :
  la méthode `power` charge davantage la marge sur les outsiders, si bien qu'une sélection penchant vers les favoris améliore le CLV
  mécaniquement. Sans effet sur le verdict : il manque environ 10 points pour couvrir la marge et la décote.

## 36. Forme récente et croisement de moyennes exponentielles contre le marché (2026-10-22)

Demande de Paul : une équipe qui surperforme depuis quelques matchs serait « sur une bonne lancée » ; analogie avec le trading (prix
au-dessus de ses EMA, croisement EMA rapide/lente). Le §35 incluait la forme aux points parmi 34 variables, jamais isolée ni mesurée
contre les attentes du marché. Étude ponctuelle en Python (venv jetable : pandas 3.1.0, numpy 2.5.4, scipy 1.18.1), script dans le
scratchpad, non versionné ; lecture seule des CSV football-data. Aucun crédit dépensé.

**Littérature** : les parieurs misent davantage sur les équipes en série (NBA, NFL), mais la cote absorbe ce biais. Ni parier avec la
série ni parier contre ne rapporte après la marge (Camerer 1989 ; Brown & Sauer 1993 ; Paul, Weinbach & Humphreys 2011). Aucune
étude en football mesurée contre la clôture de Pinnacle.

**Protocole, validé avant tout calcul.**
- Six ligues, matchs avec les cotes Pinnacle d'avant-match ; référence : probabilités sans marge `power`.
- Surperformance d'une équipe sur un match : points obtenus − (3 p_victoire + p_nul) du marché. Trois signaux, différence
  domicile − extérieur : **S3** (somme sur 3 matchs), **S6** (sur 6), **SX** (EMA α = 0,5 − EMA α = 0,1 de la surperformance).
  Remise à zéro à chaque saison ; matchs évalués seulement si les deux équipes en ont joué au moins 6 dans la saison (choix fait à
  l'écriture du script, le protocole ne le précisait pas). **Test de fuite** : signaux recalculés sur les seuls matchs antérieurs au
  2022-01-01 (11 135), identiques au bit près.
- Test d'information : p ∝ p_marché · exp(β z x), x = +1 domicile, 0 nul, −1 extérieur, z = signal divisé par son écart-type
  d'entraînement. β > 0 : élan non intégré par le marché ; β < 0 : sur-réaction du marché. β estimé sur 2017/18–2022/23
  (10 155 matchs), vérifié sur 2023/24–2024/25 (3 307).
- Test de pari : décile extrême du signal (seuils de l'entraînement), sens « élan » (parier l'équipe en forme) ou « contre » (parier
  son adversaire), cote Pinnacle d'avant-match × 0,910, CLV contre la clôture sans marge de Pinnacle.
- Bootstrap par blocs de semaines ISO, 2 000 tirages, graine 20261022. **Configurations : 3 signaux × 2 sens = 6.**

**Test d'information** (log loss du marché en vérification : 0,95878).

| Signal | β entraînement [IC 95 %] | β vérification [IC 95 %] | Δ log loss, β gelé [IC 95 %] |
|---|---|---|---|
| S3 | −0,018 [−0,046 ; +0,010] | −0,031 [−0,073 ; +0,008] | −0,00023 [−0,00065 ; +0,00016] |
| S6 | −0,019 [−0,044 ; +0,008] | −0,007 [−0,049 ; +0,037] | +0,00003 [−0,00044 ; +0,00054] |
| SX | −0,006 [−0,034 ; +0,022] | −0,028 [−0,069 ; +0,010] | −0,00009 [−0,00022 ; +0,00004] |

Les β sont tous légèrement négatifs (sens d'une sur-réaction du marché, comme dans la littérature), mais compatibles avec 0. À
β = −0,03, une équipe au décile extrême (z ≈ 1,5) perd moins d'un point de probabilité : très loin de la marge.

**Test de pari (vérification).** CLV de référence sans signal, au prix Pinnacle brut : domicile −3,5 %, nul −4,4 %, extérieur −7,0 %
(la méthode `power` charge la marge sur les outsiders) ; au prix décoté : −12,1 % à −15,4 %.

| Signal | Sens | Paris | CLV décotée [IC 95 %] | ROI décoté [IC 95 %] | CLV Pinnacle brute |
|---|---|---|---|---|---|
| S3 | élan | 614 | −12,8 % [−13,6 ; −12,0] | −22,7 % [−31,7 ; −13,0] | −4,2 % |
| S3 | contre | 614 | −14,6 % [−15,7 ; −13,6] | −15,0 % [−25,2 ; −4,8] | −6,2 % |
| S6 | élan | 645 | −13,4 % [−14,1 ; −12,7] | −18,8 % [−28,2 ; −9,2] | −4,8 % |
| S6 | contre | 645 | −13,5 % [−14,4 ; −12,7] | −24,9 % [−34,7 ; −15,0] | −5,0 % |
| SX | élan | 635 | −12,7 % [−13,4 ; −12,0] | −25,3 % [−34,1 ; −15,8] | −4,1 % |
| SX | contre | 635 | −14,4 % [−15,2 ; −13,6] | −10,1 % [−21,6 ; +2,1] | −6,0 % |

Les CLV restent dans la fourchette de la référence sans signal ; aucun ROI n'est positif. Les résultats d'entraînement vont dans le
même sens (CLV décotée de −11,9 % à −12,6 %).

**Verdict : piste close.** La forme récente, mesurée contre les attentes du marché, n'apporte aucune information que Pinnacle n'a pas
déjà. L'analogie avec le trading ne tient pas : l'EMA porte ici sur une donnée publique que le marché intègre déjà dans son prix ; le
seul équivalent du prix, le mouvement de la cote de Pinnacle, a été mesuré au §34 (inexploitable).

**Limites.** Pas de Pinnacle d'avant-match en 2025/26 et 2026/27, donc pas de vérification sur ces saisons ; surperformance mesurée
en points seulement (ni buts, ni tirs, ni xG) ; une seule fenêtre de seuil (décile).

## 37. Loto Foot (FDJ) : jeu à répartition, public biaisé, jackpots garantis (2026-10-22)

**Pourquoi cette piste.** Les marges françaises tuent tout petit avantage contre Pinnacle (§33 à §36). Un jeu **à répartition**
échappe à ce mur : les gagnants se partagent les mises des autres joueurs. L'opérateur n'a aucun intérêt à limiter un gagnant, le
jeu est anonyme en point de vente, et le prix d'une issue dépend de ce que joue le public, pas d'un bookmaker. Si le public se
trompe plus que le prélèvement, une grille choisie sur les probabilités du marché a une espérance positive (Clair & Letscher,
*Operations Research* 2007, « Optimal Strategies for Sports Betting Pools »).

**OddsPapi indisponible ce jour.** Tous les points d'accès répondent HTTP 500 « Authentication error occurred » avec notre clé ;
une clé inventée reçoit 401 `INVALID_API_KEY`. La clé existe donc, le compte côté serveur est en défaut. Le test prospectif
`edge latency` (§34) en dépend : à surveiller lundi.

**Règles** (règlement FDJ « Loto Foot 7 & 15 », PDF media.fdj.fr, version ancienne ; décision ANJ 2026-146 pour le taux ;
règlement v6 du 24 juillet 2026 non lu : site de l'ANJ en maintenance le 2026-10-22) :
- grille simple 1 € ; doubles et triples multiplient le prix ; 70 % des mises reviennent aux gagnants ;
- rangs : LF7 et LF8 45 % / 55 % (1er rang : sans faute, 2e rang : une erreur) ; LF12 et LF15 40 / 20 / 20 / 20 % ; masse d'un rang
  partagée à parts égales entre ses gagnants ;
- certaines listes ont un 1er rang **garanti** (« pactole ») : 150 000 € (LF8), 200 000 € (LF12), 500 000 € (LF15), augmenté de
  500 000 € à chaque liste LF15 sans gagnant ; le complément vient d'un fonds de l'opérateur ;
- jeu de point de vente : grille préparée dans l'application, validée chez un buraliste (à vérifier par Paul).
Ces parts sont confirmées par les rapports récents (rapprochement des masses par rang, tableau des pages de prono-match.fr).

**Sources de données.**
- Pronosoft et le site des points de vente FDJ ont tout l'historique, mais **Pronosoft appartient à FDJ United** (mentions légales :
  contact en fdjunited.com). Rien n'y a été collecté (règle du projet : pas de collecte automatisée chez un opérateur).
- prono-match.fr (tiers, `robots.txt` permissif) : grille en cours avec la **répartition officielle FDJ** par match, relevée toutes
  les heures, et rapports des 8 dernières grilles de chaque jeu. Relevé horaire lancé le 2026-10-22 (étude ponctuelle,
  `scripts/lotofoot/live_get.sh`).
- secretsdujeu.com : historique LF7 et LF15 depuis 2004, mais derrière des paramètres d'URL que son `robots.txt` interdit. Utilisé
  seulement à travers les **copies de la Wayback Machine** de la page « dernier résultat » (151 copies, 2016 à 2026).
- Cotes : notre archive The Odds API (Pinnacle) ; 3 crédits dépensés par `edge snapshot --home` dans une base séparée pour la
  Ligue 2, la Ligue Europa et la Ligue Conférence (79 sur 450) ; football-data.co.uk pour l'historique (18 ligues, 2015/16 à 2026/27).

**Modèle d'espérance** (`scripts/lotofoot/lf_ev.py`, `lf_fast.py`). Ticket simple t, issues indépendantes de probabilités p (marché
sans marge, méthode puissance). Grilles du public : S grilles d'un euro, signes choisis match par match selon la répartition q. Pour
une issue o, le nombre de concurrents à e erreurs suit une loi de Poisson de moyenne λ = S · P_q(n − e bons | o) (loi de
Poisson-binomiale). Notre part d'une masse M est M · E[1/(1 + W)] = M · (1 − e^{−λ})/λ. Espérance exacte par énumération des issues à
moins de R erreurs (4 106 issues au LF15). Contrôle : l'espérance moyenne d'une grille du public, pondérée par q, vaut **0,700**
(LF7), soit exactement le taux de retour.

**Public.** Sur 18 matchs des grilles du jour couverts par Pinnacle : q ∝ p^γ · e^{δ·nul}, **γ = 1,67**, δ = 0,075, divergence de
Kullback-Leibler moyenne 0,010. Le public surjoue les favoris et les clubs populaires (exemples du 2026-10-22 : Inter 90 % joué contre
73 % au marché, Bayern à Fribourg 87 % contre 71 %, Lyon à Hoffenheim 45 % contre 30 %, Monaco 61 % contre 45 %).

**Calibration sur 46 grilles passées** (copies Wayback, 21 LF15 et 25 LF7 entièrement appariées à football-data, 134 observations ;
protocole figé en tête de `calib.py` ; 86 grilles écartées : coupes, sélections nationales, matchs hors football-data, un
résultat discordant). Régression de Poisson des gagnants publiés sur les gagnants prédits : log E[W] = 0,14 + **0,976** log λ,
β IC 95 % [0,92 ; 1,04] (bootstrap des grilles). Rapport réel / prédit : 1,15 [0,65 ; 1,97] à λ = 1, 1,03 à λ = 100, 0,98 à λ = 1 000.
Dispersion d'une grille à l'autre : facteur log-normal d'écart-type ≈ 0,6. **Écart** : aux rangs 1 où le modèle prédit moins de
2 gagnants, 17 gagnants observés pour 7,9 prédits (les joueurs à systèmes couvrent aussi les combinaisons improbables).

**Grilles du 2026-10-22, 10 tickets distincts par grille** (κ = 1,15, β = 0,976 ; « plancher » = concurrents ajoutés à chaque
combinaison pour représenter les systèmes) :

| Jeu (mises estimées) | Espérance par euro, plancher 0 | plancher 3 | plancher 10 | P(gain) par grille |
|---|---|---|---|---|
| LF7 n°132 (≈ 160 k€) | 2,93 | 2,79 | 2,48 | 11 % |
| LF8 n°146 (≈ 520 k€, pactole 150 k€) | 3,55 | 2,99 | 2,46 | 2 % |
| LF12 n°91 (≈ 550 k€, pactole 200 k€) | 2,12 | 1,60 | 1,37 | 14 % |
| LF15 n°88 (≈ 1,27 M€, pactole 500 k€) | 1,45 | 0,98 | 0,75 | 2 % |

Grille « tous les favoris » : 1,18 (LF7), 0,60 (LF8), 0,89 (LF12), 0,29 (LF15). Les espérances changent peu avec S, car pour λ grand
l'espérance vaut environ 0,7 · p(c)/q(c). Croissance logarithmique (bankroll 1 000 €, plancher 3) : LF7 +109 ‱ par grille à 10
tickets, +237 ‱ à 40 ; LF8 +26 ‱ à 10 ; **LF12 négative** dès 2 tickets (gains trop rares pour la bankroll).

**Backtest sur les gains réellement versés : non concluant.** Mêmes 46 grilles, 10 tickets par grille choisis sur les cotes
d'avant-match et le modèle q ∝ p^1,67 (pas de répartition historique) : 0 € gagné pour 460 € (protocole en tête de
`backtest_lf.py`). Le diagnostic (`bt_diag.py`) montre que ces tickets n'avaient que **0,41 grille payée attendue** au total : sans
la vraie répartition, l'optimiseur part vers des combinaisons extrêmes (environ 1 % de chance de gain par grille), là où le modèle
extrapole. Ce backtest n'a donc aucune puissance ; il ne confirme ni n'infirme l'avantage.

**Ce qui est établi, ce qui ne l'est pas.**
- Établi : la structure (70 %, rangs, garanties), le biais massif du public, la calibration du nombre de gagnants dans la zone
  λ ≥ 1 où se trouvent les tickets LF7 et LF8 du jour.
- Non établi : un gain réalisé. La preuve demande des grilles avec la **vraie** répartition FDJ et les rapports : relevé prospectif
  (quelques semaines pour une calibration, plusieurs centaines de grilles pour un ROI), ou les copies Wayback des pages de grilles du
  site des points de vente FDJ (plus de 2 000 copies LF7 recensées, non collectées : contenu de l'opérateur).
- Configurations regardées : 1 modèle du public, 4 jeux, 4 scénarios de plancher, K = 10 (et 1 à 40 pour la croissance).

**Pistes voisines notées.** Les « Grilles » de Winamax (pari mutuel en ligne, Grille 7 à 50/50, Grille 9 à 50/25/25, Grille 12 à
40/20/20/20, dotation minimale parfois garantie, règlement des paris 2024 §9) : prélèvement, garanties et volumes ne sont publiés que
sur le site de Winamax. Une dotation garantie supérieure aux mises nettes y donnerait un gain sans modèle, à lire à la main.

**Backtest avec la vraie répartition (copies Wayback des grilles FDJ, accord de Paul du 2026-10-22).** La Wayback Machine conserve
2 260 pages de grilles du site des points de vente FDJ (938 LF7, 570 LF8, 387 LF12, 365 LF15, 2021 à 2026). Leur état JSON embarqué
donne, par match, le nombre de grilles jouées sur chaque signe (donc q et S exacts), le résultat, les gagnants et le gain de chaque
rang, et la garantie annoncée. Collecte : 3 requêtes en parallèle ; archive.org plafonne à environ 15 pages par minute par adresse,
quel que soit le nombre de requêtes ou le langage (mesuré). 1 589 grilles exploitables (496 copies sans données, 85 sans résultat,
81 illisibles ou de format ancien, 9 échecs de téléchargement).
Masse réellement versée : 72 à 77 % de S, pas 70 % (mises unitaires de 2 ou 5 €) ; le backtest garde 70 %, par prudence.
Protocole figé avant lecture dans `scripts/lotofoot/PROTOCOLE_FDJ.md` (amendements datés, tous antérieurs à la lecture du test) :
- entraînement avant 2024, test unique à partir de 2024 ;
- LF7/LF8 et LF12/LF15 ajustés séparément ;
- concurrents W ~ binomiale négative de moyenne κ λ^β + φ · S / 3^n (λ calculé avec le vrai q) ;
- 10 tickets par grille ;
- cotes d'avant-match de football-data (clôture pour les ligues « new »).

**Calibration du nombre de gagnants avec le vrai q** (`fdj_calib.py`). Le modèle d'indépendance est juste : rapport observé / prédit de
0,97 à 1,16 pour λ de 3 à 10 000.
- LF7/LF8, 1 660 observations d'entraînement : κ = 0,870, β = 1,021, φ = 0,104, dispersion relative 0,30 ; sur le test, rapport 0,96
  à 1,08 dès λ ≥ 10.
- LF12/LF15, 1 260 observations : κ = 1,080, β = 0,995, φ = 0,437. Pour λ de 0,3 à 3, le modèle sous-prédit les concurrents d'un
  facteur 1,37 : il surestime l'espérance des tickets les plus à contre-courant.

**Gains réalisés** (10 € par grille, gain = masse du rang / (gagnants publiés + nos tickets)) :

| Jeu | Grilles | Misé | Gagné | ROI [IC 95 %, semaines] | Espérance annoncée | Grilles payées | ROI sans le meilleur gain |
|---|---|---|---|---|---|---|---|
| LF7 | 252 | 2 520 € | 3 374 € | +34 % [−84 ; +257] | 1,47 | 16 | −65 % |
| LF8 | 157 | 1 570 € | 1 544 € | −2 % [−100 ; +171] | 1,90 | 3 | −81 % |
| LF12 | 25 | 250 € | 35 € | −86 % [−100 ; −65] | 2,45 | 3 | −95 % |
| LF15 | 84 | 840 € | 3 593 € | +328 % [−100 ; +1 231] | 2,73 | 1 | −100 % |
| **Tous** | **518** | **5 180 €** | **8 545 €** | **+65 % [−77 ; +262]** | 1,85 | 23 | −4 % |

- **Test figé, LF7/LF8** (90 grilles, 2024-2025) : −96,5 % ; 4,2 grilles payées attendues, 1 observée ; gain réalisé sous le
  5e centile de la loi simulée (P = 0,03). LF7/LF8 par année : 2021 −64 %, 2022 −87 %, 2023 +309 % (gains de 2 500 € et 1 244 €),
  2024 −95 %, 2025 −100 %.
- **Test figé, LF12/LF15** (32 grilles) : +1 027 % [−100 ; +3 377], dû à une seule grille (LF15 n°76 du 2025-10-25 : 4 tickets à 12
  bons sur 14, 3 593 €, calcul vérifié sur la masse publiée du rang).
- **Contre l'hypothèse nulle** « 10 tickets tirés comme le public » (même modèle de concurrents) : gain moyen −28,7 % ; probabilité
  d'atteindre le gain réalisé : 0,025 sur les 518 grilles, 0,071 sur LF7/LF8 seuls.
- Signes à contre-courant (p − q ≥ 0,05) : marché d'avant-match 0,279, clôture 0,272, fréquence réelle 0,267, public 0,205. La clôture
  est calibrée ; l'avant-match du vendredi surestime ces signes de 0,7 point. L'écart avec le public est réel et grand.

**Verdict.** Le mécanisme est établi : le public sous-joue les signes à contre-courant d'environ 6,7 points, et le nombre de gagnants
est bien prédit. La sélection fait mieux que le public (P = 0,025). Aucun gain n'est démontré pour autant : le ROI réalisé de +65 %
a un intervalle de [−77 % ; +262 %], il dépend de deux ou trois gros gains, et le test figé LF7/LF8 échoue. C'est une loterie à
espérance probablement positive : dans l'historique, 4 années LF7/LF8 sur 5 sont fortement négatives. Une année de jeu (≈ 500 grilles)
ne donnerait pas plus de précision que tout cet historique.

**Limites.**
- q et S finaux, au lieu de la répartition visible une heure avant la clôture.
- Une seule stratégie testée (10 tickets) ; aucune variante à gains plus fréquents n'a été essayée, pour ne pas fouiller le test.
- Couverture des cotes : 518 grilles sur 1 589 (sélections nationales et coupes d'Europe absentes de football-data). L'analyse
  secondaire (foule débiaisée sur les matchs sans cote, 444 grilles de test) va dans le même sens : LF7/LF8 −89 %, LF12/LF15 +134 %
  (une grille).
- 496 copies vides non remplacées par une copie antérieure de la même page.

**Optimisation de la stratégie (2026-10-22, demande de Paul : budget fixe de 10 € par semaine, objectif « espérance maximale avec au
moins environ 4 grilles payées par an »).** Calculs ex ante, sans aucun résultat de match (`frontier.py`, `robust.py`,
`year_sim.py`). Tickets gloutons maximisant EV + μ · P(payé), μ ∈ {0 ; 3 ; 10 ; 30}, k = 1 à 10 ; les 518 grilles appariées.

| Jeu | EV/€ μ=0, k=10 | P(grille payée) | EV/€ μ=30, k=10 | P(grille payée) | EV/€ μ=10, k=10, scénario S3 |
|---|---|---|---|---|---|
| LF7 | 1,40 | 8,0 % | 1,18 | 18,1 % | 1,21 |
| LF8 | 1,79 | 2,6 % | 1,72 | 5,5 % | 1,56 |
| LF12 | 2,34 | 8,0 % | 1,74 | 21,7 % | 1,76 |
| LF15 | 2,58 | 2,6 % | 2,46 | 5,3 % | 1,97 |

- Plus de tickets sur une même grille fait baisser l'espérance par euro (les meilleurs tickets passent d'abord).
- μ achète de la fréquence de gain au prix de l'espérance.
- Les garanties augmentent un peu l'espérance du LF7 (1,19 sans, 1,25 avec 100 k€) et du LF8 (1,43 sans, 1,63 avec 150 k€).
- Scénario S3 (concurrents × 1,4 pour λ < 3, et p rapproché de q de 10 %) : l'espérance baisse de 10 à 25 % et reste au-dessus de 1
  partout dans le modèle.

Année simulée (52 semaines, 10 €/semaine, tickets répartis chaque semaine sur les grilles disponibles) :

| Option | Grilles payées/an | Résultat net médian | P(année positive) | P(perte > 400 €) |
|---|---|---|---|---|
| A : LF7 seul, μ = 10 | 6,8 | −94 € (S3 −131 €) | 42 % (S3 40 %) | 12 % (S3 15 %) |
| B : tous jeux, μ = 30 | 3,8–4,0 | −277 € (S3 −316 €) | 32 % (S3 27 %) | 34 % (S3 38 %) |
| C : LF7 + LF8, μ = 30 | 4,8–4,9 | −187 € (S3 −226 €) | 38 % (S3 35 %) | 25 % (S3 28 %) |

L'espérance annuelle simulée est positive pour les trois options, de +240 € à +840 €. Mais elle est instable d'une simulation à
l'autre, car elle dépend de jackpots rares (B est plus élevé en S3 qu'en base, par le seul effet du bruit) ; la médiane et
P(année positive) sont robustes. **Dans le modèle, l'année médiane est perdante pour toutes les options.** La perte maximale est le
budget (520 €). Tout reste à confirmer en prospectif : le test figé LF7/LF8 a échoué (§37 ci-dessus).

**Sans plancher de fréquence (décision de Paul : budget indolore, seule l'espérance compte).** Résultat cumulé simulé
(`horizon_sim.py`, médiane et P(positif), 10 €/semaine) :

| Option | 1 an | 3 ans | 5 ans | 10 ans |
|---|---|---|---|---|
| Espérance max : tous jeux, μ = 0 (base) | −311 €, 32 % | −224 €, 45 % | +120 €, 52 % | +1 454 €, 63 % |
| idem, S3 | −358 €, 28 % | −503 €, 38 % | −376 €, 44 % | +363 €, 54 % |
| A : LF7 seul, μ = 10 (base) | −88 €, 43 % | +318 €, 60 % | +860 €, 67 % | +2 334 €, 80 % |
| idem, S3 | −130 €, 39 % | +146 €, 55 % | +594 €, 63 % | +1 808 €, 74 % |

L'option d'espérance maximale a une moyenne plus haute (environ +660 € par an contre +290 €), mais son surplus vient de jackpots
LF15/LF12 trop rares pour se réaliser en dix ans. Il dépend aussi de la zone la moins fiable du modèle (λ < 3, concurrents
sous-prédits). L'espérance de A vient des rangs du LF7, où la calibration est solide, et se réalise : moyenne et médiane sur 10 ans
sont proches.

**LF7 + LF8 seulement** (Paul écarte LF12 et LF15, trop proches de la loterie ; `horizon_78.py`, médiane et P(positif)) :

| Option | Moyenne/an | Part LF8 | 1 an | 3 ans | 5 ans | 10 ans |
|---|---|---|---|---|---|---|
| LF7 + LF8, μ = 0 | +490 € (S3 +386) | 62 % | −171 €, 40 % | +196 €, 54 % | +816 €, 63 % | +2 997 €, 77 % (S3 +2 107 €, 69 %) |
| **LF7 + LF8, μ = 10** | **+490 € (S3 +403)** | 64 % | −156 €, 41 % | +239 €, 56 % | +911 €, 64 % | **+2 990 €, 78 % (S3 +2 214 €, 71 %)** |
| LF7 + LF8, μ = 30 | +464 € (S3 +367) | 69 % | −193 €, 38 % | +147 €, 54 % | +712 €, 62 % | +2 735 €, 75 % (S3 +1 769 €, 68 %) |
| A : LF7 seul, μ = 10 | +295 € (S3 +246) | 0 % | −88 €, 43 % | +318 €, 60 % | +860 €, 67 % | +2 334 €, 80 % (S3 +1 808 €, 74 %) |

**Stratégie candidate retenue : LF7 + LF8, μ = 10, 10 € par semaine.** Espérance supérieure d'environ deux tiers à celle du LF7
seul ; meilleure médiane à 5 et 10 ans, y compris en S3 ; probabilité d'être gagnant inférieure de 2 à 3 points ; premières années
un peu plus creuses. À confirmer en prospectif : le test figé LF7/LF8 a échoué.

**Budget maximal de 16 € par semaine, séance du samedi** (décision de Paul : 16 € colle au prix d'une grille à 4 doubles ; c'est un
plafond, pas une obligation ; `budget16.py`, `threshold_sat.py`). Espérance par euro, avec le partage entre nos propres tickets (base / S3) :

| 16 € sur une grille | LF7 | LF8 |
|---|---|---|
| 16 simples distincts d'au moins 2 signes | 1,23 / 1,13 | 1,63 / 1,45 |
| **16 simples libres (seulement distincts)** | **1,40 / 1,29** | **1,85 / 1,62** |
| Multiple à 4 doubles (16 €) | 1,33 / 1,23 | 1,72 / 1,56 |
| Multiple à 3 doubles (8 €) | 1,33 / 1,23 | 1,81 / 1,61 |

La contrainte d'écart de 2 signes, choisie dans le protocole par souci de diversité, coûte environ 12 % d'espérance : elle est
abandonnée pour la production. Une multiple fait 5 à 7 % de moins que des simples libres, mais elle se remplit en une ligne.

Règle d'arrêt (grilles ouvertes le samedi à 15 h, 133 samedis de l'historique) : un ticket n'est joué que si son espérance S3 dépasse τ.

| τ | Dépense moyenne | Samedis sans jeu | Gain espéré par an (S3) |
|---|---|---|---|
| 1,0 | 15,0 € | 1 % | +656 € (+490 €) |
| **1,2** | **13,5 €** | 5 % | **+640 € (+481 €)** |
| 1,5 | 9,1 € | 24 % | +527 € (+400 €) |
| 2,0 | 1,9 € | 74 % | +162 € (+125 €) |

Ce tableau somme les espérances individuelles des tickets ; le partage entre nos propres tickets (quelques pour cent) n'y est pas
déduit. **Règle retenue pour le plan : tickets simples libres, μ = 10, τ = 1,2 sur l'espérance S3, 16 € au plus par samedi.**

## 38. Phase 14 — Loto Foot en production : vérifications et choix (2026-10-22)

**Règlement Loto Sports v6** (ANJ, décision 2026-PR-087 du 24 juillet 2026, PDF lu le 2026-10-22 ; décision 2026-146 du 21 juillet 2026) :
- **aucun jeu en ligne** : l'application et le site ParionsSport servent seulement à *préparer* une prise de jeu (un code), qui
  s'enregistre en point de vente (art. 3.1.5 et 3.3) ; la séance du samedi chez le buraliste reste donc la seule voie ;
- bulletin papier de 8 grilles, deux par formule (art. 3.1.2 et 3.1.3) ; nombre de grilles par code de l'application : non précisé,
  à constater sur place (repli : une grille à 4 doubles, 16 €) ;
- LF7 et LF8 : 45 % et 55 % de 70 % des mises (art. 7.6 et 10.1) ;
- pactoles (art. 10.1.7) : LF7 100 000 € les dimanches avec au moins 3 matchs de Ligue 1 (+10 000 € par liste sans gagnant) ; LF8
  150 000 € les mardis, vendredis et samedis avec au moins 5 matchs de Ligue 2, et les mercredis avec au moins 5 matchs de Ligue des
  champions (+25 000 €) ;
- **à revoir en juin 2027** : la décision 2026-146 autorise une « Option Boost » facultative (+50 % de mise) qui ajoute des gains et un
  rang ; le modèle ne la connaît pas.

**prono-match.fr** (mentions légales lues le 2026-10-22) : « toute reproduction sans autorisation est interdite ». Décision de Paul :
seuls les faits extraits sont stockés (rencontres, pourcents FDJ, résultats, rapports), jamais les pages. Les classes CSS du site sont
des identifiants générés ; la lecture s'appuie sur les textes et sur l'attribut `title` des signes. Une grille « pas encore ouverte aux
paris » est lue (clôture, pactole) sans répartition. La FDJ publie la répartition quelques heures après l'ouverture : à vérifier sur les
premiers samedis pour le LF7 du dimanche.

**Cotes des rencontres.** Clés de The Odds API vérifiées sur `/sports` (gratuit) le 2026-10-22 : 67 compétitions de football, dont la
liste `lotofoot_leagues` (31 clés vues dans les grilles LF7 et LF8 de 2025-2026 : Ligue 2, coupes d'Europe, championnats étrangers,
coupes nationales). Une grille LF7/LF8 récente compte en moyenne 1,6 compétition hors de l'archive quotidienne (Big 5 + Écosse) ; une
séance coûte donc environ 3 à 5 crédits, 8 au plus (`lotofoot_session_credits`), soit 15 à 30 par mois sur une marge d'environ 90.
`/events` (gratuit) trouve la compétition d'un match ; seules les compétitions actives selon `/sports` sont interrogées.

**Essai réel du 2026-10-22** (copie de la base) : relevé de prono-match (LF7 n°133 pas encore ouvert, LF8 n°146 avec répartition), séance :
la Ligue 2 est trouvée par `/events` et relevée (1 crédit, 80 utilisés sur 450), 16 tickets LF8 retenus, espérance défavorable de 2,36
à 3,19, probabilité de gain de 0,2 à 0,5 % chacun.

**Écarts avec le plan annoncé.** Pas de commande `settle` : le règlement se recalcule à la demande à partir du journal (`settle.rs`).
La loi simulée du gain tire seulement les issues (générateur SplitMix64 écrit dans le projet, sans dépendance) et prend le nombre de
concurrents à sa valeur attendue. Le module n'a ni générique ni exception de lint.

**Critère de vérification, figé avant le premier ticket** (`crates/core/src/lotofoot/verify.rs`) : aux points de contrôle de 26 et 52
semaines après la première séance, le modèle est en défaut si une tranche `λ ≥ 10` d'au moins 10 observations a un rapport gagnants
publiés / prédits hors de [0,8 ; 1,25], ou si le gain réalisé des tickets recommandés tombe sous le 5e centile de sa loi simulée
(20 000 tirages, graine 20 261 022). Fréquence des tickets payés : test de Poisson unilatéral, affiché. ROI : affiché, sans rôle.

**Collecte d'archive.org.** Le débit est plafonné à environ 15 pages par minute par adresse IP, qu'on envoie 3 ou 5 requêtes en
parallèle (mesuré le 2026-10-22). Un client écrit dans un autre langage n'irait pas plus vite.

## 39. Loto Foot : regroupement des tickets en grilles multiples sans perte (2026-10-23)

**Demande de Paul** : ne plus remplir 16 grilles simples par samedi quand une grille multiple ne coûte rien.

**Règlement Loto Sports v6** (PDF lu le 2026-10-22, relu ce jour) :
- une multiple porte des doubles et des triples ; sa mise est le nombre de jeux simples qu'elle contient (art. 5.2.5, 5.2.10,
  5.2.11) ;
- elle paie comme ces jeux simples : le tableau de calcul des gains (art. 10.5) donne, pour chaque mise, le nombre de gains à
  chaque rang, soit exactement celui des tickets du produit. Rencontre annulée (art. 10.6) : les gains des « pronostics de
  renvoi » sont multipliés par 2 ou 3 par double ou triple annulé, ce qui revient au même que les tickets simples (une rencontre
  annulée est gagnante pour tous les signes) ;
- formes acceptées (art. 5.2.6 et 5.2.7), en nombre maximal de doubles pour 0, 1, 2 et 3 triples : LF7 4, 3, 1, 0 ; LF8 5, 3, 2, 0.
  Au plafond de 16 €, toutes les multiples de 16 tickets au plus sont acceptées ; la règle est quand même codée ;
- bulletin papier : 2 grilles par formule (art. 3.1.3). Nombre de grilles par code de l'application ParionsSport : non précisé.

**Règle.** Les tickets ne changent pas : l'optimiseur les choisit comme avant (§37, §38), puis `lotofoot::bulletin::group`
cherche, grille par grille, la partition des tickets choisis en le **moins possible de grilles acceptées**, chaque grille étant un
produit cartésien formé uniquement de tickets choisis. Aucun ticket n'est ajouté ni retiré : mise, espérance, journal, règlement et
vérification sont inchangés (aucune migration, version du modèle `lotofoot-1` conservée). Une étude de multiples « avec perte »
(choisir d'autres tickets pour remplir moins de grilles) a été proposée puis écartée par Paul.

**Méthode.** (1) Fermeture par réunion : deux grilles égales sauf sur une rencontre, aux signes disjoints, se réunissent ; toute
grille multiple contenue dans les tickets s'obtient ainsi (on la coupe le long d'une rencontre à plusieurs signes). (2) Couverture
exacte minimale par programmation dynamique sur les sous-ensembles de tickets (2^16 masques au plus).

**Exemples** : les 16 tickets LF7 du test de référence de l'optimiseur tiennent en 6 grilles (8 €, trois à 2 €, deux à 1 €) ; les
16 tickets de la séance réelle LF8 n°146 du 2026-10-22 en 5 grilles (8 €, 4 €, 2 €, deux à 1 €).

**Affichage (2026-10-23, après la phase 15).** Les grilles à remplir figurent dans `edge lotofoot recommend`, dans l'image jointe au
message Discord (`report/lotofoot_image.rs`), dans le texte du message (une ligne par grille : signes, prix, espérance défavorable
moyenne de ses tickets) et sur la page `/lotofoot` (une colonne par grille à remplir, puis le détail des tickets : espérance,
règlement, déclaration). Les vues sont calculées par `lotofoot_page::page` (champ `lines`), partagé par la page et le message.

**Chances et gains affichés (2026-10-23, demande de Paul : l'espérance se lit mal sur des probabilités aussi basses).** Pour chaque
ticket, la chance de chaque rang (« 1 sur 2 400 ») et le gain moyen quand on l'atteint, `E[gain | rang] = E[gain · 1{rang}] / P(rang)`,
calculés par le même parcours que l'espérance (`GridModel::rank_odds`). Pour chaque grille à remplir, les mêmes deux rangs selon son
meilleur ticket (`GridModel::line_odds`, énumération des 3^n issues) : « sans faute » si l'un de ses tickets est parfait, avec le gain
de toute la grille (ses tickets à une erreur compris), puis « une erreur ». Un premier essai donnait un seul gain moyen « quand elle
paie », mélange du sans-faute et d'une erreur : Paul l'a jugé trompeur, il est retiré. Probabilités du marché, modèle de base ; les séances passées sont recalculées à partir
des entrées figées au journal (`GridModel::from_journal`, déplacé de `verify.rs`). Le partage entre nos propres tickets n'est pas
déduit, comme dans l'espérance.

## 40. Phase 15 — Application Web à la place d'edge-gui (2026-10-23)

**Demande de Paul** : servir une vraie application Web plutôt qu'une page statique, en reprenant l'interface d'edge-gui ;
consultation sur le PC fixe de la maison (pas de rendu téléphone), le portable restant serveur ; edge-gui retiré une fois le serveur
en place ; une image dans le message Discord des opportunités et du Loto Foot (celle-ci « ressemblant à un ticket »).

**Choix : rendu côté serveur, sans script**, plutôt qu'egui compilé en WebAssembly. egui dans le navigateur aurait demandé une API
JSON et des appels HTTP asynchrones côté navigateur (contraire à la règle « pas d'async »), une crate de types séparée (le cœur, avec
`rusqlite` en C, `ureq` et `resvg`, ne compile pas pour le navigateur) et l'assouplissement de la politique de sécurité. Aucune
dépendance nouvelle : `tiny_http` (§28) et `minijinja` suffisent ; les graphiques sont des SVG écrits à la main, colorés par les
classes CSS de la page (thème clair ou sombre du système).

**Organisation.** Module `web` du cœur : routage, connexion, formulaires, pages, testés sans ouvrir de port ; la CLI ne fait que
traduire HTTP ↔ `web::Request`/`Response`. Les écritures passent par les fonctions du cœur qu'utilisaient la CLI et edge-gui
(`betting::ledger`, `bonus::tracking`, `lotofoot::played`) : mêmes règles (cote minimale, règlement unique, `off_model`).
Palette commune (`report/theme.rs`) : celle d'edge-gui, partagée par le site, le rapport quotidien et les images Discord.

**Sécurité** (le site écrit désormais au journal) :
- secret `serve_token` (fichier en 0600) ou `EDGE_SERVE_TOKEN`, 24 caractères au moins, lettres, chiffres, `-` et `_` (il finit
  dans un cookie) ; `edge serve` refuse de démarrer sans lui ;
- cookie de session `HttpOnly; SameSite=Strict`, un an ; comparaison du secret en temps constant ;
- tout `POST` doit porter un en-tête `Origin` égal à `http://` + `Host`, sinon 403 ; sans session, toute adresse renvoie à
  `/connexion` (sauf les polices) ;
- politique de sécurité : `default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src 'self'; form-action 'self';
  frame-ancestors 'none'; base-uri 'none'` ;
- HTTP sans chiffrement : secret et cookie circulent en clair, acceptable sur le réseau de la maison seulement.

**Constaté en essayant.**
- `Referrer-Policy: no-referrer` (§28) fait envoyer `Origin: null` par Chromium avec un formulaire : le contrôle d'origine
  refusait toutes les connexions (journal du serveur, 2026-10-23). Passé à `same-origin` : aucune adresse du site n'est transmise
  à un site extérieur, et l'origine redevient visible pour le site lui-même.
- L'échappement HTML de `minijinja` écrit `/` en `&#x2f;` et `'` en `&#x27;` : sans effet pour le navigateur, mais les tests
  décodent ces entités avant de chercher un texte (`web/testing.rs`, `unescape`).
- Le test du serveur restait bloqué : la réponse binaire de la police était lue avec `read_to_string`, qui échoue, et le thread du
  serveur attendait ensuite une requête qui ne venait plus. Corrigé en lisant des octets. Au même moment, le corps n'est plus lu
  que pour un `POST` (borné à 16 Kio) : un `GET` n'en a pas, inutile de le lire.

**Images Discord.** Recommandations : inchangées, aux couleurs de la palette sombre. Loto Foot (`report/lotofoot_image.rs`) :
un bulletin par grille, une ligne par rencontre, une colonne par grille à remplir (multiples de §39), cases 1, N, 2 à cocher
noircies, prix de chaque grille ; rendu par `resvg` avec Inter, comme l'autre image. Séance LF8 n°146 : 5 colonnes (8, 1, 2, 4,
1 €).

**Retiré** : `crates/gui`, `eframe`, `egui_plot` (204 paquets en moins dans `Cargo.lock`) et `deploy/edge-gui.desktop`.

## 41. Loto Foot : séance à la demande depuis l'application Web (2026-10-23)

**Demande de Paul** : pouvoir lancer l'optimisation plus tôt que le samedi midi (courses du matin), avec une mise réglable.
Choix de Paul : séance réelle écrite au journal, mise de 1 à 16 €, Discord en option.

- Bouton « Optimiser maintenant » sur `/lotofoot` (`POST /lotofoot/optimiser`, champs `mise` et `discord`) : même chemin que
  `edge lotofoot recommend` (`lotofoot::live::recommend` : relevé frais de prono-match, cotes manquantes, séance, Discord). La mise
  est un plafond en euros entiers, de 1 € au plafond gelé `lotofoot_max_tickets` (16 €, §37) ; elle est enregistrée dans les
  paramètres de la séance (`max_tickets`). Seuil τ, poids μ et modèle inchangés.
- **Le bouton recalcule et remplace ; le timer ne remplace jamais** (choix de Paul, même jour, après un premier essai où « une seule
  séance par grille » l'empêchait de refaire l'optimisation à 10 € : la répartition des autres joueurs bouge, il faut pouvoir
  recalculer). Le bouton recalcule aussi les grilles déjà recommandées, sauf celles dont des tickets sont déjà déclarés joués ; sa
  séance remplace leur recommandation, même sans y retenir de ticket (grilles listées dans `replaces` de ses paramètres). Le timer
  écarte les grilles déjà recommandées (« déjà recommandée »).
- **Seule la dernière recommandation de chaque grille compte** (`store::current_recos`), pour le bilan, la vérification figée et la
  déclaration des tickets joués. Le journal reste immuable : les anciennes lignes y restent, ignorées. Aucune fuite du futur : une
  grille n'est recalculée qu'au moins 3 h avant sa clôture. Effet sur le journal réel : les séances d'essai n°1 et n°2 du
  2026-10-22 (mêmes 16 tickets du LF8 n°146) ne comptent plus qu'une fois (la n°2).
- La page et « J'ai joué » prennent les tickets en vigueur de la semaine de la dernière séance (`store::week_recos`, séances des
  6 jours qui la précèdent) : un clic le matin puis le timer de midi sur une nouvelle grille se jouent ensemble.
- **Une séance sans ticket et sans remplacement n'est pas écrite** : elle n'a rien à journaliser. Ses raisons restent dans le message Discord (désormais listées quand il n'y a aucun ticket),
  dans la console et dans le message de la page.
- Durée : 10 à 20 secondes (5 s de pause entre deux pages de prono-match, puis les cotes) ; le serveur traite les requêtes une à
  une, les autres pages attendent pendant ce temps. Coût : 1 crédit par compétition manquante, 8 au plus par séance.

## 42. Recherche élargie : jeux à répartition garantie et concours dotés, hors paris sportifs (2026-10-23)

**Demande de Paul** : chercher, au-delà des paris sportifs, un système automatisable et rentable « à la manière du Loto Foot ».
Critère retenu : une somme **garantie** (jackpot forcé, prix fixe) répartie entre un nombre de participants qui varie, ou un
concours gratuit doté. Recherche documentaire seulement, aucune mesure ; chiffres non vérifiés marqués comme tels.

**Pistes ouvertes, par ordre d'intérêt**

1. **Grand Loto FDJ (jackpot forcé).** Selon la FDJ, si personne ne trouve la combinaison, les 20 M€ « sont redistribués sur les
   rangs de gain inférieurs » au lieu d'être reportés ([FDJ, Grand Loto du 26/06/2026](https://www.fdj.fr/mag/actus/article-grand-loto-20M-euros-260626),
   [Grand Loto de Noël 2025](https://www.fdj.fr/mag/actus/article-grand-loto-de-noel)). Grille à 5 € et 100 codes à 20 000 €
   ([tirage-gagnant.com](https://tirage-gagnant.com/114708/grand-loto-des-vacances-2026/), source non officielle). Plusieurs
   éditions par an (été, Noël…). Espérance par grille, avec `J` le jackpot, `C` les codes, `N` le nombre de grilles et `r` le gain
   moyen des rangs inférieurs hors jackpot :

       E = (J + C) / N + r ;   E > 5 €  ⇔  N < (J + C) / (5 − r).

   Avec `r ≈ 1 €` (hypothèse à mesurer), le point mort est vers 5,5 millions de grilles. Seul volume trouvé : 4 millions de grilles
   en 2017 ([Economie Matin](https://www.economiematin.fr/?p=52768), ancien et non officiel). Comme au Loto Foot, le choix de
   numéros impopulaires (au-delà de 31, hors suites et dates) augmente la part reçue quand le jackpot est partagé (Thaler et Ziemba,
   « Parimutuel Betting Markets: Racetracks and Lotteries », *J. Econ. Perspectives*, 1988). Données : la FDJ publie l'historique en
   ZIP, Grand Loto compris, avec le nombre de gagnants par rang ([historique Loto](https://www.fdj.fr/jeux-de-tirage/loto/historique)),
   d'où `N` se déduit par les rangs à forte fréquence. **Mesurable sans frais.**
2. **EuroMillions, cinquième tirage au plafond.** Au plafond (250 M€), le jackpot est remis en jeu au plus quatre fois, puis
   « redistribué au(x) gagnant(s) du deuxième rang » au cinquième tirage ([FDJ](https://www.fdj.fr/mag/questions/article-plafond-euromillions)).
   Espérance de ce rang, avec `p₂ = 1/6 991 908` (5 numéros et 1 étoile) et `λ = N·p₂` :
   `250 M€ · (1 − e^(−λ)) / N`, soit environ 2,50 € par grille si `N = 10⁸`. Le cas **ne s'est jamais produit**
   ([lottery.co.uk](https://www.lottery.co.uk/euromillions/jackpot-cap)) : alerte possible, pas un système.
3. **Tournoi de bots FutureEval (Metaculus).** Concours gratuit de prévision par des bots LLM : 50 000 $ par saison, trois saisons
   par an, plus une MiniBench à 1 000 $ toutes les deux semaines ([annonce Fall 2026](https://forum.effectivealtruism.org/posts/hrJrsgMCwsdBuAqgQ/announcing-fall-2026-futureeval-bot-tournament)).
   Printemps 2026 : 111 bots hors Metaculus, 297 questions ([bilan](https://www.lesswrong.com/posts/wZBbDqzfBjYG58CxK/futureeval-spring-results-pros-beat-bots-but-the-gap-is)).
   Saison d'automne ouverte depuis le 28/09/2026, entrée possible jusqu'à quelques semaines avant le 01/01/2027 (départ à 0 point).
   Crédits LLM financés sur demande (environ 100 $ au départ, non garantis). Particuliers éligibles ; bots commerciaux (trois
   personnes ou plus) exclus des prix sauf code ouvert. Commentaires de raisonnement et questionnaire de saison obligatoires pour
   les prix. Modèle de départ : [metac-bot-template](https://github.com/Metaculus/metac-bot-template) (Python). **Non trouvé** :
   formule de répartition des prix (page Metaculus en 403). Prix imposables en France (à vérifier).
4. **Bonus de bienvenue hippiques** (agrément distinct du sport, absent de `docs/bonus-bienvenue.md`) : Betclic Turf (1er pari
   hippique perdant remboursé en freebet, 100 € max, rejouable sur le sport), PMU Turf (100 € de bonus turf et un ticket Quinté+),
   Genybet (jusqu'à 200 €, dont 10 % des mises reversées) ([vitepromo](https://www.vitepromo.com/en/divers-rencontres/code-promo-betclicturf),
   [vitepromo Genybet](https://www.vitepromo.com/divers-rencontres/code-promo-genybet), comparateurs non officiels, juin-juillet 2026).
   Ponctuel, sans couverture possible en pari mutuel.

**Pistes écartées**

- **Quinté+ et sa tirelire** : structure proche du Loto Foot, mais retour aux joueurs d'environ 72 % (site affilié, non vérifié ;
  pourcentages masqués dans les décisions ANJ [2024-120](https://anj.fr/sites/default/files/2024-07/D%C3%A9cision%202024-120_IP_Quint%C3%A9%2B_PMU_PDV.pdf)) ;
  la répartition des combinaisons jouées par le public n'est pas publiée, et les données en direct viennent du site du PMU, un opérateur.
- **Sorare** : régime JONUM (loi SREN, décret 2026-60) ; selon une [lecture du décret](https://medium.com/@N00V0/sorare-and-the-jonum-decree-b9efbf80a5d7)
  non vérifiée sur Légifrance, aucune récompense en monnaie légale. Spéculation sur des cartes.
- **Numerai** : gains liés à une mise en NMR (crypto-actif) brûlée en cas de mauvaise performance ; aucun avantage démontré.
- **Concours de pronostics gratuits** : surtout des lots ; les dotations en argent viennent d'opérateurs et sont ponctuelles.
- **Loto ordinaire** : jackpot redistribué seulement après 34 tirages sans gagnant ([FDJ, 2022](https://www.fdj.fr/mag/questions/article-plafond-loto)) ; jamais atteint en pratique.

## 43. Grand Loto et EuroMillions au plafond : mesures sur l'historique officiel FDJ (2026-10-23)

**Données** : archives ZIP de la FDJ, téléchargées avec l'accord de Paul depuis les pages
[historique Loto](https://www.fdj.fr/jeux-de-tirage/loto/historique) et
[historique EuroMillions](https://www.fdj.fr/jeux-de-tirage/euromillions-my-million/historique) (lien
`sto.api.fdj.fr/anonymous/service-draw-info/v3/documentations/…`) : `grandloto_201912.csv` (9 tirages, 12/2019 → 12/2025),
`lotonoel2017.csv` (2017, 2018), `loto_201911.csv`, `euromillions_*.csv` (2004 → 20/10/2026). Séparateur `;`, décimales à la
virgule, nombre de gagnants et rapport par rang ; pour l'EuroMillions, gagnants en France **et en Europe**. Le Grand Loto du
26/06/2026 (20 M€, deux gagnants) n'y figure pas encore. Scripts ponctuels dans le scratchpad, non versionnés.

**Méthode.** Le nombre de grilles `N` se déduit des rangs à forte fréquence (estimateur de Poisson
`N = Σ n_k / Σ p_k`, rangs 5 à 9 au Loto, 9 à 13 à l'EuroMillions ; `p_k` dénombré sur 5/49 × 1/10 et 5/50 × 2/12). Contrôle sur
les Loto ordinaires de 2026 : 2,6 à 5,1 millions de grilles par tirage, 0,7 à 0,9 € reversé par grille de 2,20 € hors jackpot,
cohérent. Les rangs 8 et 9 pris seuls donnent l'écart d'estimation (entre parenthèses ci-dessous). Biais possible : un tirage de
numéros populaires gonfle les gagnants, donc `N`.

**Grand Loto (jackpot forcé, 5 € la grille).** Le jackpot `J` est toujours versé, au premier rang ou aux rangs inférieurs : le
total versé divisé par `N` est donc l'espérance par grille du tirage, et non un résultat chanceux. Avec `r` le reversement des
rangs inférieurs hors jackpot et codes (2,1 à 2,6 €, soit environ 48 % de la mise), le point mort est `N* = (J + C) / (5 − r)`.

| Tirage | J (M€) | N estimé (M) | N* (M) | Espérance par grille |
|---|---|---|---|---|
| 22/12/2017 | 20 | 7,41 | 8,49 | +8 % (prix de 2017 non vérifié) |
| 25/12/2018 | 15 | 8,82 | 6,45 | −14 % |
| 24/12/2019 | 15 | 7,38 | 6,59 | −6 % |
| 25/12/2020 | 19 | 7,58 | 8,16 | +4 % (−6 à +19) |
| 24/12/2021 | 15 | 9,35 | 6,58 | −15 % |
| 25/12/2022 | 15 | 6,99 | 6,88 | −1 % |
| 24/12/2023 | 15 | 7,26 | 6,82 | −3 % |
| **26/07/2024 (JO)** | 15 | **2,86** | 5,80 | **+60 %** (+45 à +87) |
| 24/12/2024 | 20 | 8,40 | 8,09 | −2 % |
| **04/07/2025 (été)** | 20 | **4,94** | 8,66 | **+38 %** (+26 à +48) |
| **25/12/2025** | 20 | 6,84 | 9,01 | **+16 %** (+12 à +17) |

Lecture : les éditions **d'été** attirent deux à trois fois moins de grilles, avec le même jackpot garanti : espérance de +38 à
+60 %. Les éditions de Noël à 20 M€ sont autour de l'équilibre ; à 15 M€, elles perdent. Prochaine : **Noël, vendredi
25/12/2026, 20 M€, 5 €** ([tirage-gagnant.com](https://tirage-gagnant.com/jouer-grand-loto-de-noel), non officiel ; jeu en ligne
sur fdj.fr possible). Avec `N` entre 6,8 et 8,4 millions, l'espérance va de +1 % à +13 %. L'espérance est réelle, mais la variance
est celle d'une loterie : presque toute la valeur vient du jackpot et des rangs 2 à 4. Pas encore mesuré : le gain des numéros
impopulaires (estimable sur les quelque 1 000 Loto depuis 2019, par régression du nombre de gagnants sur les numéros tirés).

**EuroMillions, cinquième tirage au plafond.** Grilles européennes : médiane de 21 à 26 millions par tirage, 60 à 67 millions aux
pointes au plafond. Au plafond, les rangs 2 à 13 reversent environ 0,85 à 1,0 € par grille de 2,50 € (0,4 à 0,5 € d'ordinaire), car
l'excédent du jackpot descend aux rangs inférieurs. Épisodes à 250 M€ : 2025-03 (gagné au premier tirage au plafond), 2025-06
(gagné au quatrième), 2025-08 ; aucun en 2026. En 2022 (230 M€), le jackpot a aussi été gagné au quatrième tirage au plafond.
Espérance d'un cinquième tirage, avec `p₂ = 1/6 991 908` et `r ≈ 0,9 €` :

    V(N) = 250 M€ · (1 − e^(−N·p₂)) / N + r

| N (millions) | 70 | 100 | 150 | point mort |
|---|---|---|---|---|
| V par grille de 2,50 € | 4,47 € (+79 %) | 3,40 € (+36 %) | 2,57 € (+3 %) | environ 156 M |

Probabilité qu'un épisode atteigne le cinquième tirage : `Π e^(−N_i/139,8 M)` sur quatre tirages ; avec 50 à 65 millions de grilles,
environ 0,15 à 0,25. Avec deux à trois épisodes par an, un tel tirage arriverait **une fois tous les deux à quatre ans** ; il n'a
jamais eu lieu. Une alerte suffit : déclenchée quand le quatrième tirage au plafond n'a pas de gagnant.

## 44. Phase 16 — Grand Loto et alerte EuroMillions : vérifications, choix et premières mesures (2026-10-23)

**Choix de Paul (2026-10-23)** : relevé automatique de l'archive FDJ (fichiers publics, sans compte), tout en Rust, 20 € par Grand
Loto (quatre grilles à 5 €), grille jouée si ses euros rendus par euro atteignent **1,05 au nombre de grilles pessimiste** (80e
centile de la prévision), ligne de commande et Discord sans page Web, éditions saisies dans `edge.toml`. Dépendance `zip` 8.6.0
validée (même version et mêmes options que celle de `calamine` : aucun paquet ajouté à `Cargo.lock`).

**Règlements lus à la source**
- Loto, décision ANJ [2026-PR-023](https://www.anj.fr/sites/default/files/2026-02/D%C3%A9cision%20n%C2%B0%202026-PR-023_HOM%20R%C3%A8glement_Loto2dTirage_PDV-Ligne.pdf) :
  grille de Grand Loto à 5 € (art. 5) ; 54,85 % des mises aux gagnants, « défini globalement et non tirage par tirage » ; au Grand Loto,
  5,5 % au rang 1 et 2,85 % au Fonds de Super Cagnotte ; rang 9 fixe à 5 € ; clé des rangs 2 à 8 propre au Grand Loto (5,06 % à
  40,26 %) ; **art. 8.3.4** : sans gagnant, le jackpot va aux rangs 2 à 9 (1,28 %, 0,77 %, 3,48 %, 1,31 %, 5,96 %, 11,23 %,
  **51,05 % au rang 8**, 24,92 % au rang 9) ; jackpot minimum de 20 M€. Loto ordinaire : report limité à 34 tirages.
- EuroMillions, décision ANJ [2025-PR-054](https://anj.fr/sites/default/files/2025-03/D%C3%A9cision%202025-PR-054_Homologation_R%C3%A8glement_Euromillion-Ligne_PDV-3.pdf) :
  1,10 € par grille aux gagnants (art. 4.2.2.1) ; clé des rangs 2 à 13 (2,61 % au rang 2…, 40 % au total) ; plafond de 200 M€ en
  2020, plus 10 M€ par cycle, au plus 250 M€ ; excédent au rang inférieur ayant un gagnant (4.2.5.5) ; **cinquième tirage
  consécutif au plafond sans gagnant : le jackpot va au rang inférieur** (4.2.5.6). Le règlement prévoit aussi des « Tirages Super
  Jackpot Forcément Gagné » annoncés par avis sur fdj.fr : **non détectés** par l'outil (il faudrait lire fdj.fr).
- Décision ANJ [2026-136](https://anj.fr/sites/default/files/2026-07/D%C3%A9cision%20n%C2%B02026-136_LFDJ_AU_EuroMillions_PDV%26Ligne.pdf) :
  EuroMillions modifiée pour 2027 (« retour à l'ancienne matrice de jeu » ; date, prix et plafond masqués). L'alerte refuse de
  conclure si les gagnants des rangs 12 et 13 ne collent plus à la matrice 5/50 et 2/12 (écart de plus de 30 %).

**Archives FDJ** (pages « Historique » de fdj.fr, liens `sto.api.fdj.fr/anonymous/service-draw-info/v3/documentations/<id>`) :
Loto depuis mars 2017 (`…afn6`, `…afo6`, `…afp6` en cours), Grand Loto (`…aff6`, `…afg6` en cours), EuroMillions depuis
septembre 2016 (`…afc6`, `…afd6`, `…afe6` en cours). Le Loto d'avant mars 2017 a six rangs et l'EuroMillions d'avant
septembre 2016 onze étoiles : non lus. Formats : `;`, virgule décimale, espaces de milliers, Latin-1 (anciens) ou UTF-8,
dates `JJ/MM/AAAA`, `JJ/MM/AA` ou `AAAAMMJJ`, colonnes repérées par leur nom. L'archive en cours de l'EuroMillions est
datée du soir du tirage (21 h 36) : timer à 22 h 30. Le Grand Loto du 26/06/2026 n'est pas encore dans l'archive.

**Méthode** (`crates/core/src/loto/`)
- Nombre de grilles : `N̂ = Σ_{k=5..9} n_k / Σ p_k` (EuroMillions : rangs 9 à 13). Prévision d'une édition : `ln N` des cinq
  dernières éditions antérieures de la même saison (été, Noël), loi de Student à `n − 1` degrés, écart type au moins 0,15.
- Espérance d'une grille : clés du règlement, partage `E[1/(1+M)] = (1 − e^{−μ})/μ` avec les autres gagnants. Contrôle : une
  grille au hasard retrouve à 0,5 % près la forme fermée `(J + C)/N + 5 € × 46,5 %`.
- Préférences du public : modèle produit (un poids par numéro, une part par chance), vraisemblance multinomiale des gagnants
  par rang (le nombre de grilles du tirage s'y simplifie), L2 d'écart type 0,5, L-BFGS à gradient analytique. **Critère fixé
  avant la mesure** : gain de log-vraisemblance hors échantillon par tirage dont l'intervalle à 95 % (bootstrap par semaines)
  exclut 0, **et** pente de calibration du rang 8 dont la borne basse dépasse 0,5. Walk-forward annuel, années de test 2019 à 2026.
- Grilles : parmi les 15 numéros et les 3 chances les plus délaissés, motifs écartés (trois numéros consécutifs, écart
  constant ; Baker & McHale 2011), au plus deux numéros communs entre deux grilles. Sans préférences validées : grilles au
  hasard, reproductibles (graine = date de l'édition).

**Mesures sur l'archive réelle (2026-10-23)** : 1 507 Loto, 11 Grand Loto, 1 051 EuroMillions.
- Grand Loto passés : identiques au §43 (contrôle des tests).
- Préférences, walk-forward : gain de **5 071 [4 697 ; 5 475]** de log-vraisemblance par tirage, positif chaque année
  (4 065 à 6 103) ; pente du rang 8 **0,93 [0,92 ; 0,94]** : **critère rempli**. Contrôles hors critère : pente 0,94 à deux
  numéros trouvés, 0,93 [0,91 ; 0,96] à trois, **0,88 [0,83 ; 0,93] à quatre** : le modèle prédit bien, mais exagère d'environ 10
  % les écarts des rangs 3 et 4. Une seule configuration de modèle a été essayée.
- Les plus joués : 7, 13, 12, 11, 9, 10, 5 (poids 1,26 à 1,39) ; les plus délaissés : 46, 36, 40, 39, 35, 32, 41 (0,73 à 0,77) ;
  chance 7 jouée par 14,2 % des grilles, chance 1 par 7,7 %. Conforme à la littérature (dates de naissance, 7 et 13).
- Contrôle sur les 11 Grand Loto (préférences ajustées sur les Loto antérieurs) : gain moyen d'environ +1 000 par tirage, mais
  négatif 4 fois sur 11 (−1 426 à Noël 2025) : **le public du Grand Loto choisit moins comme celui du Loto**. Non validé à part
  (trop peu de tirages).
- À la référence (Noël à 20 M€, 7 millions de grilles) : 5,46 € rendus par une grille au hasard, **7,49 €** par la grille la plus
  délaissée (`35-36-39-40-46+1`, six fois moins jouée qu'une grille moyenne). Le gain vient surtout des rangs 2 à 6 (quatre et
  trois numéros), où une telle grille a deux à trois fois moins de cogagnants.
- Séance à blanc pour Noël 2026 (20 M€) : 7,71 millions de grilles prévues, 9,00 au pessimiste ; grille au hasard 5,18 €
  (4,77 € au pessimiste : **ne pas jouer**) ; grilles délaissées 6,97 à 7,14 € (6,49 à 6,66 € au pessimiste : **jouer**).
  **La décision de Noël dépend donc entièrement des préférences**, et précisément des rangs où la pente est la plus basse.
- EuroMillions : 16 tirages au plafond depuis 2020, tous détectés (excédent de 6,3 à 30,5 M€) ; aucun autre tirage au-dessus de
  1 M€. Aucune alerte due au 23/10/2026.

**Limites connues** : public du Grand Loto différent ; grilles choisies à l'extrême de l'impopularité, là où le modèle
extrapole le plus (effet de sélection : le gain réel sera plus petit que prévu, de l'ordre de la pente) ; motifs et « grappes »
de numéros non modélisés au-delà du filtre ; probabilité qu'aucun autre joueur ne gagne le jackpot prise au hasard ; reports
d'un rang vide ignorés ; gains des codes non suivis ; « Tirages Super Jackpot Forcément Gagné » non détectés ; règles 2027 de
l'EuroMillions à revoir. Variance : celle d'une loterie, presque tout vient des rangs 1 à 4.

## 45. Phase 17 — Page « Grand Loto » de l'application Web (2026-10-23)

**Demande de Paul** : une vraie page `/loto`, comme `/lotofoot`. Choix de Paul : **pas de bouton** pour lancer la séance (elle reste
celle du timer, trois jours avant l'édition, jamais refaite), et **un petit formulaire** pour déclarer une grille saisie à la main.

- `GET /loto` (lecture seule, base ouverte en lecture seule) : prochaine édition de `grand_loto_editions` (la configuration n'est lue
  qu'au démarrage d'`edge serve` : une édition ajoutée demande un redémarrage) ; séance de l'édition, ou la plus récente, lue dans
  le journal telle qu'elle a été décidée (notes JSON : prévision, grille au hasard, verdict des préférences, grilles évaluées) ;
  grilles jouées et leur règlement ; série EuroMillions au plafond en cours (`loto::alert::current_streak`, même calcul que
  l'alerte) et alertes envoyées ; Grand Loto passés (`report::loto_report::edition_rows`, même calcul que `edge loto study`).
- `POST /loto/jouee` : comme `edge loto played --last-session`. `POST /loto/grille` : grille (cinq numéros puis la chance, toute
  écriture), édition, mise ; rattachée à la recommandation identique de l'édition, sinon hors modèle (invariant 7).
- Rien de neuf dans le cœur du modèle ; aucune dépendance, aucune migration.

## 46. Phase 18 — Suivi en direct des grilles Loto Foot (ESPN, 2026-10-23)

**Demande de Paul** : un code couleur sur `/lotofoot` pour voir d'un coup d'œil les tickets encore en vie pendant les matchs. The Odds
API `/scores` (2 crédits par compétition et par appel avec `daysFrom`) est refusée : le budget mensuel n'y suffirait pas.

**Sources écartées.**
- Sofascore : répond HTTP 403 à tout client qui n'est pas un navigateur, `robots.txt` compris. Y accéder voudrait dire contourner sa
  protection : exclu.
- Flashscore : `robots.txt` interdit `/direct/` et `/termines/` ; les données passent par un flux non documenté et signé. Exclu.
- prono-match : la page de grille n'affiche aucun score en cours (relevés du 2026-10-22) ; seuls les résultats officiels, après la grille.

**ESPN, API publique non documentée** (`https://site.api.espn.com/apis/site/v2/sports/soccer/{ligue}/scoreboard?dates=AAAAMMJJ`),
gratuite, sans clé, testée le 2026-10-23. Conditions d'utilisation non lues (le `robots.txt` du domaine renvoie « Access Denied ») :
usage personnel à très faible débit, même zone grise que prono-match. Elle peut changer sans préavis : la page affiche alors un
message et le règlement reste celui des résultats officiels.
- Couverture : 36 des 37 compétitions utiles (Big 5, Écosse et les 31 de `lotofoot_leagues`). `pol.1` (Ekstraklasa) répond HTTP 400
  « Failed to get events endpoint » : pas de suivi pour la Pologne.
- `dates` est un jour **de New York** : un match brésilien à 00:00 UTC le 18 figure sous le 17. Les plages `AAAAMMJJ-AAAAMMJJ`
  renvoient zéro match (essai sur la Carabao Cup) : une page par jour.
- Date d'un match sans secondes (`2026-10-24T18:45Z`). Champ `period` absent avant le coup d'envoi.
- État : `status.type.state` (`pre`, `in`, `post`), `completed`, `name` (`STATUS_SCHEDULED`, `STATUS_FIRST_HALF`, `STATUS_HALFTIME`,
  `STATUS_IN_PROGRESS`, `STATUS_SECOND_HALF`, `STATUS_FULL_TIME`, `STATUS_FINAL_PEN`…), `period` (1 et 2 : temps réglementaire ;
  5 : tirs au but). Un `STATUS_FINAL_PEN` donne le score avant les tirs, sans `linescores` : le score à 90 minutes d'un match prolongé
  n'est pas lisible. Ces matchs restent « à confirmer » jusqu'au résultat officiel.
- Écart constaté : PSG – Lyon figure chez The Odds API le 25 octobre à 19:45 UTC, chez ESPN le 26 à 19:30 UTC. La page signale alors
  la rencontre « absente de la page ESPN ».

**Choix de Paul (2026-10-23).**
1. Pas de timer : les scores sont relevés à l'affichage de `/lotofoot`, au plus une fois par minute et par page ESPN (compétition et
   jour), seulement pour les rencontres commencées depuis moins de 6 h et pas finies, des grilles de la semaine sans résultat
   officiel. La page se recharge seule toutes les 60 s (`<meta http-equiv="refresh">`, sans script) tant qu'une rencontre se joue.
2. Couleurs provisoires : un match en cours compte avec son score du moment (cases en tirets).
3. Dernier score seulement (migration 17, tables remplacées à chaque relevé) : exception assumée à l'archive immuable, le règlement
   restant fondé sur `lotofoot_result`.

**Appariement.** Compétition et coup d'envoi viennent de l'appel `/odds` figé avec la recommandation (`odds_calls` des entrées). Les
noms d'ESPN passent par la section `[ESPN]` de `config/aliases.toml` (nom ESPN → nom de The Odds API) : un nom inconnu laisse la
rencontre sans score et la page le nomme ; jamais d'appariement approximatif. La table est embarquée dans le binaire : un alias ajouté
demande de réinstaller `edge` et de relancer `edge serve`.

**Code couleur.** Par ticket, et par grille à remplir (son meilleur ticket : une rencontre n'y est fausse que si aucune case cochée
n'est sortie) : gris avant le premier signe connu, vert sans faute, orange à une erreur (2e rang encore possible), rouge au-delà ;
« perdu » dès deux erreurs définitives, « gagné » quand tout est définitif. Sous chaque grille multiple : tickets encore en lice.

**Essai réel du 2026-10-23** (copie de la base, LF8 n°146 joué, 16 tickets) : les 8 rencontres appariées (Ligue 2, Ligue 1, Serie A,
Premier League), quatre pages ESPN, page servie en 0,3 s.

## 47. Loto Foot : budget de 16 € par semaine sur toutes les grilles, seuil τ = 1,7, alerte à la sortie (2026-10-24)

**Demande de Paul.** Il peut jouer tous les jours ; il sort environ 3,2 LF7 et 3,4 LF8 par semaine (numérotation FDJ de 2025-2026 ;
5,1 et 2,3 en 2022). Avec la séance unique du samedi (§37, τ = 1,2), rien ne limitait la semaine : un clic le vendredi puis le timer du
samedi pouvaient engager 32 €. Un simple plafond hebdomadaire au premier arrivé épuiserait le budget sur les premières grilles venues ; le
bon outil est un seuil relevé, le plafond n'étant qu'un garde-fou.

**Étude** (`scripts/lotofoot/weekly_budget.py`, protocole figé en tête du script, calculs ex ante, aucun résultat de match) : les 409
grilles LF7/LF8 appariées de l'étude FDJ (vraie répartition), 16 tickets de la recherche de production par grille, évalués en base et en
S3 ; 40 000 semaines simulées (Poisson λ7 = 3,2, λ8 = 3,4, grilles tirées avec remise, ordre d'arrivée aléatoire) ; sur chaque grille, les
tickets d'espérance S3 ≥ τ dans ce qui reste des 16 € de la semaine. Critère : gain net espéré S3 par semaine, maximal sur l'entraînement
(grilles closes avant 2024, 319) ; test (2024-2025, 90 grilles) regardé après le choix. Configurations : 31 valeurs de τ (1,0 à 4,0), puis
5 valeurs autour du choix sur le test et au rythme de 2022.

| τ | Dépense / semaine | Gain espéré S3 / an | Gain base / an | EV S3 par € | Semaines au plafond | Sans jeu |
|---|---|---|---|---|---|---|
| 1,2 (règle du samedi) | 15,9 € | +526 € | +704 € | 1,63 | 99 % | 0 % |
| 1,5 | 15,6 € | +654 € | +865 € | 1,81 | 95 % | 1 % |
| **1,7** | **13,9 €** | **+700 €** | **+921 €** | **1,97** | 76 % | 4 % |
| 1,9 | 9,8 € | +589 € | +772 € | 2,16 | 41 % | 16 % |
| 2,2 | 4,1 € | +309 € | +403 € | 2,44 | 10 % | 52 % |

Test, τ = 1,7 : 12,8 € par semaine, +644 € par an (S3) ; rythme de 2022 : +642 €. Le maximum est plat de 1,5 à 1,8. Sans plafond, τ = 1,2
engagerait 88 € par semaine. Limites : grilles de championnat seulement (coupes et sélections absentes), partage entre nos propres tickets
non déduit, semaines réelles non indépendantes ; espérances du modèle, aucun gain démontré (le test figé du §37 a échoué).

**Règles en production depuis le 2026-10-24** (choix de Paul) :
- `lotofoot_threshold = 1,7` (gelé) ; `lotofoot_weekly_budget = 16` : une séance ne recommande que ce qui reste du budget de la semaine
  de chaque grille (lundi-dimanche, heure de Paris, selon la clôture), en comptant les tickets recommandés **en vigueur** écrits depuis le
  24 octobre à minuit (`session::weekly_budget_since`). Exception voulue par Paul : les 16 tickets du LF8 n°146 joués le 23 restent dans
  le suivi et la vérification, mais ne comptent pas dans le budget de la semaine du 19 octobre (pour jouer le LF7 n°133).
- **Alerte à la sortie** : le relevé horaire (`edge lotofoot fetch --notify`) signale sur Discord chaque grille vue pour la première fois,
  avec son heure de clôture et son pactole, sans ticket : la répartition des autres joueurs n'est pas encore représentative. Les alertes
  envoyées sont notées au journal (migration 18, table `lotofoot_alert`, `lotofoot::alert`) : une grille encore ouverte et pas encore
  signalée l'est au prochain relevé, quel que soit le relevé qui l'a vue en premier (une séance à blanc du 24 octobre avait « volé »
  la première apparition du LF8 n°147 à une détection fondée sur la nouveauté) ; une alerte qui échoue est retentée l'heure suivante.
- **Garde-fou « répartition pas encore représentative »** : une grille dont un signe est affiché à 0 % est écartée. Le LF8 n°147,
  relevé quelques minutes après son ouverture (nuit du 23 au 24 octobre), affichait des parts de 95 / 5 / 0 % et 0 / 64 / 36 % ; le
  modèle, qui compte une part nulle comme 0,5 %, annonçait une grille à 8 € gagnant sans faute une fois sur 54 pour ≈ 49 500 €. Aucune
  des 1 109 répartitions finales LF7/LF8 de l'historique FDJ n'a de part sous 0,5 % (une seule sous 1 %). Ce garde-fou n'écarte que
  les grilles à peine ouvertes ; la dérive des heures suivantes reste à mesurer (tâche du lundi 26 octobre).
- Relevé : l'en-tête d'une grille sans pactole finit par la clôture (« clôture samedi 24 octobre à 17h55. ») ; le point final faisait
  échouer la lecture de l'heure, et le relevé horaire du LF8 n°147 a échoué de 22 h à minuit le 23 octobre. Corrigé (`pronomatch.rs`).
- Table d'alias : 7 clubs du LF8 n°147 ajoutés à la section `LOTOFOOT` (noms bruts de The Odds API relevés dans l'archive).
- **Calcul au dernier moment** : Paul lance la séance au guichet (« Optimiser maintenant »), juste avant de valider. Marge minimale avant
  la clôture : 10 minutes (`lotofoot_min_minutes_before_close`, au lieu de 3 heures pour la séance du samedi). Le message de la séance
  donne le budget restant de la semaine.
- **Timer du samedi midi retiré** : il calculait trop tôt (répartition encore loin de la finale : sur le LF8 n°146, les favoris ont gagné
  4 à 5 points dans les dix dernières heures) et consommait le budget de la semaine. `edge lotofoot recommend` reste disponible à la main.

## 48. Loto Foot : points de vente et heure limite réelle de validation (2026-10-24)

**Constat de Paul** : la plupart des buralistes ferment le dimanche, jour où ferment environ 20 % des LF7 (148 sur 741 dans
l'historique FDJ, presque toujours vers 14 h : 100 cas, puis 17 h : 29) et 6 % des LF8. Une grille ne se valide qu'au guichet
(règlement v6, art. 3.1.5) : l'heure de clôture ne dit donc pas jusqu'à quand on peut jouer.

**Recherche** (annuaires publics, 2026-10-24 ; le localisateur de la FDJ n'a pas été lu : pas de navigation automatisée chez un
opérateur) : à Cesson-Sévigné, Le Longchamp (centre commercial Beausoleil) est le seul tabac trouvé ouvert le dimanche, de 8 h 30 à 13 h
(Bottin, sans mention de la FDJ) ; Le Cessonnais, Mag Presse et Clopinette sont fermés le dimanche. À Rennes, le Relay de la gare est
ouvert tous les jours, le dimanche de 7 h à 22 h 45, et se déclare détaillant FDJ (PagesJaunes, fiche du 2026-10-02). La fiche du Tabac
Presse Titeux (Thabor, dimanche 14 h – 20 h selon un résultat de recherche) n'a pas pu être lue (refus du site).

**Règle** : `[[lotofoot_outlets]]` dans `edge.toml` (hors dépôt : les lieux disent où habite Paul) donne les horaires de la semaine de
chaque point de vente. L'alerte « nouvelle grille » indique, pour chacun, le dernier moment pour valider : l'heure de fermeture du
guichet ou la clôture de la grille, la plus proche des deux, en remontant au besoin aux jours précédents (`lotofoot::outlets`). Le calcul
et le seuil ne changent pas.

**Effet sur la stratégie, à mesurer** : valider tôt (dimanche matin pour une grille de 14 h, samedi pour une grille du dimanche) éloigne
le calcul de la répartition finale, sur laquelle le modèle et le seuil τ = 1,7 ont été calibrés. La mesure de la dérive avant la
clôture (tâche du lundi 26 octobre) dira à partir de combien d'heures l'écart devient gênant ; selon elle, il faudra choisir pour les
grilles du dimanche entre le trajet jusqu'au guichet ouvert, un seuil plus haut quand on valide tôt, ou ne pas les jouer.

## 49. Loto Foot : abaque du seuil τ selon le budget mensuel (2026-10-24)

**Demande de Paul.** Le τ = 1,7 du §47 a été choisi pour un plafond de 16 € par semaine ; le seuil optimal dépend du budget. Étude en
trois dimensions (τ, budget mensuel, gain), budgets jusqu'à 400 € par mois. Choix de Paul avant le calcul : critère « espérance, risque
affiché » (le gain espéré croît toujours avec le budget tant que le dernier ticket vaut plus de 1 €), plafond par **mois calendaire**.

**Constat qui a motivé la demande** (séance n°8 du 24 octobre) : le classement commun des tickets de toutes les grilles ouvertes a donné
les 16 € au LF7 n°133, calculé 27 h avant sa clôture (meilleur ticket à 10,7 en S3), et remplacé par rien les 16 tickets du LF8 n°147
(3,19), qui fermait deux heures plus tard. L'étude du §47 alloue grille par grille dans l'ordre d'arrivée ; la production ne le fait pas.
Décision de Paul : garder la séance n°8 et corriger la règle après la mesure de la dérive (lundi 26 octobre).

**Étude** (`scripts/lotofoot/budget_tau.py`, protocole figé en tête du script ; données et environnement Python dans
`~/.local/share/edge/etudes/`, hors dépôt, reliés par le lien `scripts/data`) : les 409 grilles LF7/LF8 du §47 (espérances retrouvées
à l'identique), 16 tickets par grille ; 2 000 tirages par grille des issues et des gagnants concurrents (binomiale négative gelée,
partage entre nos tickets compris) ; mois simulés au rythme 2025-2026, règle séquentielle, plafond mensuel sans report ; 4 000 années,
mêmes arrivées et mêmes tirages pour toutes les configurations. Configurations : 31 τ (1,0 à 4,0) × 20 budgets (20 à 400 €), soit 620 ;
τ* maximise l'espérance S3 sur l'entraînement (319 grilles closes avant 2024) ; test (90 grilles) lu après le choix.

| Budget / mois | τ* | Dépense / mois | Espérance S3 / an | Année en perte | Médiane / an | Repli 9e décile | Test : espérance S3 au τ* | Test : τ* propre |
|---|---|---|---|---|---|---|---|---|
| 20 € | 2,0 | 18,7 € | +284 € | 71 % | −206 € | 237 € | +266 € | 2,0 |
| 60 € | 1,8 | 57,6 € | +727 € | 61 % | −251 € | 697 € | +687 € | 1,8 |
| 100 € | 1,7 | 94,7 € | +1 098 € | 56 % | −224 € | 1 092 € | +1 018 € | 1,6 |
| 160 € | 1,6 | 148,6 € | +1 562 € | 52 % | −101 € | 1 566 € | +1 378 € | 1,5 |
| 200 € | 1,5 | 190,4 € | +1 829 € | 49 % | +68 € | 1 867 € | +1 586 € | 1,4 |
| 300 € | 1,4 | 271,3 € | +2 360 € | 44 % | +578 € | 2 419 € | +1 878 € | 1,3 |
| 400 € | 1,2 | 359,0 € | +2 715 € | 40 % | +1 052 € | 2 862 € | +2 254 € | 1,1 |

Tableau complet (20 budgets) et carte de chaleur : artefact « Abaque Loto Foot » (privé, https://claude.ai/artifact/6a3fyt8PqoNhuhHxxeAx9b).

**Lecture.**
- τ* descend régulièrement avec le budget ; le maximum est étroit (à 3 % près, un ou deux pas de τ autour de τ*).
- τ = 1,7 reste à moins de 3 % de l'optimum de 80 à 120 € par mois ; au-delà, il plafonne la dépense vers 128 € par mois et l'espérance
  vers +1 476 € par an (54 % de l'optimum à 400 €).
- Le test confirme la forme : τ* propre au test à 0,1 ou 0,2 près du τ* d'entraînement, espérance au τ* de 80 à 94 % de celle de
  l'entraînement ; les grilles récentes offrent moins de tickets au-dessus de τ (dépense plus faible à même τ).
- Rendement marginal décroissant : de 20 à 40 € par mois, chaque euro de plus rapporte environ 1 € d'espérance ; de 380 à 400 €,
  environ 0,17 €.
- Risque sous le modèle : la probabilité d'une année en perte baisse quand le budget monte (71 % à 20 € par mois, 40 % à 400 €), la
  médiane annuelle devient positive vers 180 € par mois, mais le repli dans l'année croît presque comme la dépense annuelle.

**Limites** : répartition finale et non celle de l'heure du calcul ; coupes et sélections absentes ; 16 tickets au plus par grille
(aux gros budgets, τ* ≈ 1,2 en tient compte sans lever la contrainte) ; mois réels non indépendants ; espérances et risques du modèle,
aucun gain démontré (le test figé du §37 a échoué). La moyenne simulée dépasse l'espérance analytique de 10 à 25 % (dispersion des
concurrents) : le critère reste l'espérance analytique, plus prudente.

**Aucune règle de production ne change ici.** Passer à un plafond mensuel, changer τ ou l'allocation entre grilles demande un plan
séparé et le go de Paul, après la mesure de la dérive.

## 50. Dérive de la répartition avant la clôture : la Wayback Machine ne suffit pas (2026-10-24)

**Question** (Paul, 2026-10-24) : repérer les espérances aberrantes selon l'heure du calcul (une espérance très haute n'est pas
aberrante 30 minutes avant la clôture). Source envisagée : des copies Wayback d'une même grille prises avant sa clôture. Accord de Paul
pour l'index CDX seulement, sans relevé en masse.

**Décompte** (index CDX, interrogé à nouveau sans regroupement le 2026-10-24 : 2 152 copies LF7 et 1 440 LF8, comme l'index relevé le
22 octobre à une copie près) : 1 508 pages LF7/LF8, dont 1 187 avec heure de clôture connue (`endValidationDate`). **12 grilles
seulement** ont une copie antérieure à la clôture (2023 à 2025), **aucune n'en a deux** ; ces copies sont prises de 20 à 92 heures avant la
clôture (médiane 51 h), une seule dans les trois dernières heures. Les pages de prono-match (`prono-match.fr/loto-foot-7/`, `-8/`) n'ont
aucune copie (88 copies du site en tout, aucune de Loto Foot).

**Conclusion** : aucun historique de la dérive n'existe dans la Wayback Machine. La seule source est notre propre relevé horaire de
prono-match (`lotofoot_split`, depuis le 22 octobre 2026 : 3 grilles, 18 à 32 relevés chacune) ; au rythme d'environ 6,6 grilles par
semaine, il faudra plusieurs semaines pour une courbe de dérive par heure avant la clôture.

## 51. Loto Foot : budget mensuel saisi dans l'application, seuil τ tiré de l'abaque, allocation par ordre de clôture (2026-10-24)

**Demande de Paul** : saisir seulement un budget mensuel dans l'application et obtenir la sélection, sans les problèmes de
chevauchement constatés le 24 octobre (séance n°8 : le LF7 n°133, calculé 27 h avant sa clôture, avait pris tout le budget au LF8
n°147). Choix de Paul, tous validés le 2026-10-24 :
- **Budget mensuel** (mois calendaire, heure de Paris, selon la clôture des grilles), de 20 à 400 €, saisi dans l'application
  (« Enregistrer le budget », ou avec « Optimiser maintenant ») ; historique immuable (migration 19, `lotofoot_budget`) ;
  `lotofoot_monthly_budget` (60 € par défaut) sert tant que rien n'est saisi. Entrée en vigueur **tout de suite** : octobre compte
  les tickets en vigueur recommandés depuis le 24 octobre (le LF8 n°146 du 23 reste hors budget, §47).
- **Seuil τ** = τ* de l'abaque du §49 au **palier inférieur** (70 € → τ* de 60 €), table gelée `lotofoot::abaque` (version
  `abaque-2026-10-24`) ; les réglages `lotofoot_threshold`, `lotofoot_weekly_budget` et `lotofoot_max_tickets` disparaissent.
- **Ordre de clôture** : à chaque séance, toutes les grilles ouvertes non jouées sont recalculées ; la plus proche de la clôture
  prend d'abord ses tickets au-dessus de τ (16 au plus par grille), la suivante le reste du mois. Les tickets déclarés joués ne sont
  jamais recalculés. C'est la règle séquentielle simulée au §47 et au §49.
- **Âge minimal provisoire de 8 h** (`MIN_GRID_AGE_HOURS`) : une grille vue depuis moins de 8 h est évaluée mais n'engage pas de
  budget. À revoir avec l'étude de dérive (`scripts/lotofoot/PROTOCOLE_DERIVE.md`, bilan vers le 5 décembre).
- Chaque séance note au journal le budget, τ, la version de l'abaque, la règle d'allocation et l'âge minimal.

**Application** (`/lotofoot`) : la séance et les tickets sont séparés. En tête, le budget du mois (engagé, restant) et τ ; puis
**À jouer** (grilles ouvertes, tickets pas encore déclarés joués, bouton « J'ai joué cette grille » par grille), **En cours**
(tickets joués, résultats attendus, suivi des scores), et **Dernière séance** (grilles évaluées, écartées et pourquoi). Un ticket joué
dont la grille a tous ses résultats n'est plus affiché : il ne compte plus que dans le bilan. Les tickets jamais joués d'une grille close
disparaissent aussi (ils restent dans le suivi du modèle).

**Correctif du soir** (avant la clôture du LF8 n°147) : l'ordre de clôture a d'abord été installé seul, à 17 h 33 ; la séance n°11
(budget de la semaine, 16 €) a donné 15 tickets au LF8 n°147 (1,80 à 2,25 en S3) et 1 au LF7 n°133 (11,98). Paul a joué les 15 tickets
du LF8 (déclarés joués, n°151 à 165).
