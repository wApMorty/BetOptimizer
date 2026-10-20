# edge — moteur de recommandations de paris sportifs

> Nom de travail. Ce fichier est chargé à chaque session et prime sur tes habitudes par défaut.
> S'il te semble faux, ou si les données le contredisent, dis-le-moi par une question au lieu de t'en écarter.

## Ce que je construis

Un outil personnel qui estime des probabilités de résultats sportifs, les confronte aux cotes des
bookmakers français agréés (Winamax, Betclic) et me **recommande** des paris quand l'écart justifie
une mise. **Le programme ne parie jamais lui-même** et n'automatise aucune interaction avec les sites
des bookmakers (pas de scraping, pas de navigation automatisée) : je place, ou non, chaque pari moi-même.

Objectif : le profit. Conséquence : rien n'est un « avantage » tant qu'il n'a pas été mesuré hors
échantillon contre le marché.

Ordre des sports : **football** (Big 5 + Premiership écossaise), puis **tennis**, puis **NBA**.
L'architecture est prévue pour les trois ; on implémente un sport à la fois.

Point de départ méthodologique (travail antérieur sur des pronostics LoL) : marge retirée des cotes,
calibration mesurée (Brier, log loss), Elo, simulation.

## Contexte français (vérifié en octobre 2026)

- Paris uniquement chez des opérateurs agréés par l'ANJ. Esport exclu : ces paris sont illégaux en France.
- Le retour aux joueurs est plafonné à 85 % par an et par agrément : les marges françaises sont élevées, la barre est haute.
- Selon l'ANJ, un opérateur ne peut pas limiter les mises d'un parieur au motif qu'il gagne ; seuls des motifs légitimes le permettent (jeu excessif, fraude, plafond uniforme annoncé). Un avantage prouvé reste donc exploitable.
- Pinnacle n'a plus d'API publique depuis juillet 2025 ; FBref n'a plus les xG d'Opta depuis début 2026.

## Mon profil de développeur

Senior Software Engineer (Python, TypeScript, infra/Kubernetes). **Je débute en Rust** et je serai
surtout relecteur. Le risque n'est pas que le code soit faux, c'est qu'il soit trop malin pour que je
le maintienne. Je veux comprendre chaque phase : le code doit être lisible et expliqué.

## Règles Rust (strictes)

- Édition 2024, workspace Cargo. `cargo fmt` et `cargo clippy --all-targets -- -D warnings` propres à chaque commit.
- **Pas d'`async` ni de tokio.** HTTP bloquant (`ureq`) ; threads standard si besoin.
- **Pas d'`unsafe`.**
- **Génériques et traits au strict minimum.** Types concrets par défaut. Un trait seulement quand deux
  implémentations existent réellement (ex. : `MatchModel` pour Poisson et Dixon-Coles).
- Pas de builders, pas de couches d'indirection, fonctions courtes.
- **Newtypes** pour les grandeurs du domaine (`Odds`, `Probability`, `Euros`) : le compilateur interdit
  de confondre une cote et une probabilité. Explique le pattern à sa première apparition.
- Erreurs : `thiserror` dans `core`, `anyhow` dans les binaires. Aucun `unwrap()` hors des tests ;
  `expect("raison")` seulement pour un invariant démontré.
- **Commentaire obligatoire** dès qu'un choix d'ownership, d'emprunt, de lifetime ou de `clone()` n'est
  pas évident pour un débutant : explique le *pourquoi*, pas le *quoi*.
- `///` sur chaque élément public, `//!` en tête de chaque module (son rôle en deux ou trois lignes).
- Identifiants en anglais ; commentaires, documentation et messages de commit en français.
- Toute formule statistique implémentée porte en commentaire sa forme mathématique et sa source.

## Pédagogie

- `LEARNING.md` (index) et `docs/learning/phase-N.md` rédigé à la fin de chaque phase :
  1. concepts Rust rencontrés, avec renvoi `fichier:ligne` ;
  2. concepts statistiques, avec formule et intuition ;
  3. trois questions pour vérifier que j'ai compris ;
  4. les trois fichiers à relire en priorité, et pourquoi.
- Première apparition d'un concept Rust (`?`, `impl From`, itérateurs, `match` exhaustif, emprunt dans
  une boucle, trait externe…) : explication courte dans le chat **et** dans le fichier de phase.

## Stack imposée

| Besoin | Crate | Remarque |
|---|---|---|
| CLI | `clap` (derive) | |
| Config | `serde`, `toml` | |
| CSV / JSON | `csv`, `serde_json` | |
| HTTP | `ureq` | bloquant |
| Base | `rusqlite` (feature `bundled`) | migrations maison : `PRAGMA user_version` + SQL embarqué |
| Dates | `chrono`, `chrono-tz` | stockage UTC, affichage Europe/Paris |
| Chemins | `directories` | aucun chemin en dur |
| Logs | `tracing`, `tracing-subscriber` | |
| Lois statistiques | `statrs` | |
| Optimisation | `argmin` + `argmin-math` (backend `Vec<f64>`) | maximum de vraisemblance ; implémenter ses traits est un usage légitime |
| Rapport | `minijinja` + `plotters` (SVG) | HTML autonome, lisible sur téléphone |
| GUI (phase 6) | `eframe`/`egui`, `egui_plot`, backend `wgpu` | |
| Tests | `#[cfg(test)]`, `approx` | |

À éviter : tokio, reqwest, le SDK Rust `the-odds-api` (asynchrone), Polars (environ 30 000 matchs :
des structs typés et du SQL suffisent et restent lisibles). Toute nouvelle dépendance : une ligne de
justification, que je valide.

## Architecture

```
edge/
├── CLAUDE.md, LEARNING.md
├── config/edge.example.toml      # aucun secret
├── config/aliases.toml           # correspondance des noms d'équipes entre sources
├── deploy/systemd/               # service + timer utilisateur (laptop CachyOS)
├── docs/learning/, docs/sources.md
└── crates/
    ├── core/   # domain, ingest, store, market, models, eval, betting, backtest, report, notify
    ├── cli/    # binaire `edge`
    └── gui/    # egui (phase 6)
```

- `core` ne connaît ni la CLI ni l'interface ; `cli` et `gui` passent uniquement par l'API de `core`.
- Secrets (clé API, URL de webhook) : variables d'environnement (`EDGE_ODDS_API_KEY`…) ou fichier de
  configuration hors dépôt, en permissions 0600. Jamais dans git.
- Exécution planifiée : timer systemd **utilisateur** sur mon laptop CachyOS (lingering activé,
  `Persistent=true`). Le cœur doit aussi compiler sous Windows.

## Données (budget : gratuit uniquement)

- **Historique foot** : CSV de football-data.co.uk (résultats, cotes d'une vingtaine de bookmakers,
  cotes de clôture). Les cotes d'avant-match y sont relevées le vendredi après-midi (week-end) et le
  mardi après-midi (semaine). À VÉRIFIER : colonnes Pinnacle après juillet 2025 ; sinon repli sur la
  moyenne de clôture.
- **Cotes courantes** : The Odds API, offre gratuite (500 crédits par mois). Bookmakers : `winamax_fr`,
  `betclic_fr` et `pinnacle` figurent tous trois dans la région `eu` (Winamax et Betclic aussi dans
  `fr`) ; les cotes Pinnacle y viennent de son site public, avec un délai.
  Coût d'un appel `/odds` = marchés × régions ; `/events` est gratuit ; l'historique coûte 10× et
  n'est pas dans le budget.
  À VÉRIFIER : clés exactes des ligues (`GET /v4/sports`, gratuit), décompte du paramètre
  `bookmakers`, en-têtes de quota.
- **Garde de budget** : chaque appel journalise les crédits restants lus dans les en-têtes ; l'outil
  refuse tout appel qui ferait dépasser un plafond mensuel configurable (défaut : 450). Garde de coût,
  pas de comportement.
- **Archive** : chaque instantané de cotes est conservé, horodaté, jamais écrasé. Les cotes historiques
  Winamax et Betclic n'existent pas gratuitement : notre archive sera la seule source. **Elle démarre
  en phase 1.**
- **Noms d'équipes** : table d'alias versionnée. Un nom inconnu bloque l'ingestion avec un message
  clair ; jamais d'appariement approximatif silencieux.
- **Tennis** (phase 7) : données de Jeff Sackmann (licence CC BY-NC-SA 4.0, usage personnel), cotes
  historiques de tennis-data.co.uk ; The Odds API ne couvre que les Grands Chelems, ATP/WTA 1000 et 500.
- **NBA** (phase 8) : sources à instruire en début de phase ; cotes historiques payantes, donc archive propre.

## Invariants méthodologiques (non négociables)

1. **Aucune fuite du futur.** Une prédiction n'utilise que des données antérieures à son horodatage.
   Validation exclusivement walk-forward ; jamais de K-fold aléatoire. Un test dédié le vérifie.
2. **Le marché est l'adversaire.** Référence : probabilités sans marge de Pinnacle (à défaut, consensus
   des bookmakers). Un modèle n'a d'avantage que s'il bat cette référence hors échantillon en log loss
   et en RPS, avec intervalle de confiance (bootstrap par blocs de journées).
3. **Calibration avant précision.** Sélection des modèles sur log loss, RPS et calibration, jamais sur
   le taux de bons pronostics.
4. **Cotes réalistes.** Un backtest décide sur les cotes disponibles au moment de la décision (relevé
   d'avant-match), jamais sur la clôture, jamais sur les colonnes « Max ». Ces cotes historiques venant
   de bookmakers étrangers, appliquer une **décote française** : écart mesuré entre Winamax/Betclic et
   Pinnacle sur notre archive dès qu'elle suffit ; avant cela, hypothèse prudente et documentée.
5. **CLV d'abord.** Métrique principale en production : écart entre la cote obtenue et la cote de
   clôture sans marge de Pinnacle. ROI en second, toujours avec intervalle de confiance.
6. **Journal immuable.** Chaque recommandation est enregistrée à sa création (horodatage, version et
   paramètres du modèle, instantané de cotes, probabilité, avantage, mise) et n'est jamais modifiée.
7. **Paris hors modèle marqués.** Mes paris réels sont journalisés à part. Tout pari qui ne correspond
   pas à une recommandation est marqué `off_model`, exclu des métriques du modèle et rapporté
   séparément : mes choix face à ceux du modèle.
8. **Seuils gelés.** Seuils (avantage minimal…) et hyperparamètres réglés sur les saisons
   d'entraînement et de validation, gelés avant la saison de test. Le rapport indique combien de
   configurations ont été essayées.
9. **Reproductible.** Ajustements déterministes ; version du modèle stockée avec chaque prédiction.

## Mises

- Kelly, fraction configurable. **Défaut : demi-Kelly (0,5), mon choix du 2026-10-20** (en Kelly complet, un avantage
  annoncé surestimé fait décroître la bankroll : `docs/sources.md` §27). Le backtest affiche
  côte à côte 1, ½ et ¼ Kelly : croissance logarithmique, drawdown maximal, probabilité de ruine
  (Monte Carlo).
- Au plus une sélection par match pour les recommandations du modèle. Les cotes boostées (seules ou cumulées avec une
  recommandation) peuvent porter plusieurs sélections sur un même match : mises dimensionnées sur l'exposition conjointe
  (Kelly sur scénarios de score, positions ouvertes comprises), jamais en traitant ces paris comme indépendants. Décision du 2026-10-20.
- Paris simultanés : maximiser l'espérance du logarithme de la bankroll sur toutes les combinaisons
  d'issues (énumération exacte jusqu'à 12 paris, optimisation `argmin`) ; au-delà, approximation
  documentée. La somme des mises ne dépasse jamais la bankroll.
- Mise arrondie aux 0,10 € inférieurs ; ignorée sous la mise minimale configurée.
- Chaque recommandation affiche la **cote minimale** en dessous de laquelle le pari cesse d'être recommandé.

## Pièges connus

- Promus et début de saison : peu de données, d'où la régularisation.
- Écosse : la ligue se scinde en deux groupes de six après 33 journées ; surveiller la calibration en
  fin de saison (matchs sans enjeu).
- Matchs reportés ou annulés : recommandation annulée, pari remboursé.
- Fuseaux horaires : tout stocker en UTC.
- Cotes aberrantes (somme des probabilités implicites hors de [1,00 ; 1,30]) : signalées, jamais
  corrigées en silence.

## Feuille de route

Chaque phase : plan court → mon go → code et tests → résumé, fichier pédagogique, **arrêt**.

- **Phase 0 — Fondations.** Workspace, configuration, migrations, `edge init`, lint et tests, squelette de `LEARNING.md`.
- **Phase 1 — Archive des cotes.** Client The Odds API bloquant écrit à la main, garde de budget,
  instantanés quotidiens Big 5 + Écosse (marché 1X2 ; Pinnacle, Winamax, Betclic), noms bruts conservés,
  timer systemd. Plan de consommation mensuel chiffré.
- **Phase 2 — Historique et marché.** Ingestion football-data (saisons à fixer avec moi), table
  d'alias, contrôle qualité, retrait de marge (multiplicatif, puissance, Shin : choisir sur la
  calibration), calibration du marché par ligue et par tranche de cote (biais favori/outsider, Écosse
  comprise), premier rapport HTML.
- **Phase 3 — Modèles.** Poisson puis Dixon-Coles (correction ρ des scores faibles, pondération
  temporelle ξ, régularisation L2 pour les promus), ajustement par ligue, walk-forward hebdomadaire,
  grille de scores → 1X2, plus/moins, les deux équipes marquent. Comparaison au marché. Test de
  référence contre `penaltyblog` (Python) si Python est disponible.
- **Phase 4 — Backtest et mises.** Détection de valeur, Kelly (1, ½, ¼), Kelly simultané, CLV
  historique, ROI avec intervalle de confiance, drawdown, gel des seuils.
- **Phase 5 — Recommandations quotidiennes.** Prédictions du jour, filtrage, mises, instantané
  Pinnacle juste avant le coup d'envoi pour la CLV (timer fréquent mais sans coût : il n'appelle
  l'API que si un match recommandé commence dans les 30 minutes), notification téléphone (**demande-moi Discord ou
  Telegram avant d'implémenter**) avec le rapport HTML joint, `edge bet log`, règlement des paris.
- **Phase 6 — Application egui.** Recommandations, saisie des paris, courbe de bankroll, calibration.
- **Phase 7 — Tennis.** Elo par surface (mélange général/surface).
- **Phase 8 — NBA.** Elo avec marge de victoire, repos et back-to-back ; écart et total par approximation normale.

## Manière de travailler

- Début de phase : plan (fichiers, dépendances justifiées, tests), puis **attendre mon go**.
- Vérifier à la source (documentation d'API, colonnes CSV, versions de crates) avant de coder ;
  consigner dans `docs/sources.md`, avec la date.
- Commits atomiques ; fmt, clippy et tests verts avant chaque commit.
- Tests obligatoires pour les mathématiques : probabilités sans marge qui somment à 1, grille de scores
  qui somme à 1, Kelly nul sans avantage, symétrie de l'Elo, absence de fuite du futur.
- Fin de phase : résumé en dix lignes maximum, commandes pour essayer, limites connues, fichier
  pédagogique, puis **stop**.

## Communication avec moi

Français soutenu, sans contractions orales, tutoiement. Réponses courtes et précises.
Pas d'interpolation : si une consigne est ambiguë, demande. Ne valide pas tout : signale un problème
par une question précise.

## Références

- Walsh & Joshi, calibration contre précision : https://arxiv.org/pdf/2303.06021
- penaltyblog, implémentation de référence (Poisson, Dixon-Coles) : https://penaltyblog.readthedocs.io/en/latest/models/overview.html
- The Odds API, bookmakers par région : https://the-odds-api.com/sports-odds-data/bookmaker-apis.html
- ANJ, limitation des mises : https://www.anj.fr/sites/default/files/2026-06/D%C3%A9lib%C3%A9ration%20n%C2%B0%202021-C-01_LimitationMisesParieurs%20modifi%C3%A9e.pdf
- Données tennis de Jeff Sackmann : https://github.com/JeffSackmann/tennis_atp