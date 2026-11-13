# Planification (systemd utilisateur)

Prérequis : lingering activé (`loginctl enable-linger $USER`) et binaire installé dans `~/.local/bin`.

| Timer | Quand | Ce qu'il fait | Crédits |
|---|---|---|---|
| `edge-snapshot` | 14 h (Paris) | relevé des cotes de 6 ligues (`edge snapshot`) | 6 par jour |
| `edge-recommend` | dès la fin du relevé de 14 h | recommandations du jour et notification Discord (`edge recommend --notify`) ; pas de timer, lancé par `edge-snapshot` s'il réussit (`OnSuccess=`) | 0 |
| `edge-nba` | 22 h UTC | cotes de la NBA (vainqueur, handicaps, totaux) et, un jour sur deux, résultats (`edge nba snapshot`) | 3 par jour + 2 tous les deux jours |
| `edge-closing` | toutes les 10 minutes | clôture de Pinnacle des matchs suivis qui commencent dans 30 minutes (`edge closing`) | 1 par match suivi, 0 sinon |
| `edge-latency` | lundi 9 h | historique OddsPapi des matchs terminés et verdict du test de latence (`edge latency fetch`, `edge latency report`, sources §34) | 0 (6 requêtes OddsPapi sur 250 par mois) |
| `edge-lotofoot-fetch` | toutes les heures (minute 7) | grilles LF7 et LF8, répartition FDJ, résultats et rapports sur prono-match ; alerte Discord à la sortie de chaque grille, avec son heure de clôture (`edge lotofoot fetch --notify`, sources §38 et §47) | 0 |
| `edge-backup` | 14 h 30 (Paris) | copie datée et vérifiée de la base dans `backup_dir`, rotation sur `backup_keep` copies (`edge backup`, sources §73) | 0 |
| `edge-loto` | 22 h 30 (Paris) | historique FDJ, alerte EuroMillions au plafond, séance du Grand Loto dû dans les 3 jours (`edge loto fetch --notify`, `edge loto recommend --notify`, sources §44) | 0 |

Service permanent (pas un timer) : `edge-serve`, l'application Web (voir plus bas). Aucun crédit.

Unité modèle (ni timer ni service permanent) : `edge-notify-failure@.service`. Les sept services à exécution unique et
`edge-backup` l'appellent par `OnFailure=` quand ils échouent ; elle envoie une alerte Discord (`edge notify-failure <unité>`),
au plus une par unité toutes les six heures. Il n'y a rien à activer : il suffit que le fichier soit copié.

```sh
# Installer ou mettre à jour le binaire
cargo install --path crates/cli --locked --root ~/.local

# Installer et activer les timers
mkdir -p ~/.config/systemd/user
cp deploy/systemd/edge-*.service deploy/systemd/edge-*.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now edge-snapshot.timer edge-closing.timer edge-nba.timer edge-latency.timer \
    edge-lotofoot-fetch.timer edge-loto.timer

# Sauvegarde : seulement une fois `backup_dir` renseigné dans ~/.config/edge/edge.toml (voir « Sauvegarde » plus bas)
systemctl --user enable --now edge-backup.timer

# Une seule fois, après le passage de edge-recommend en OnSuccess= (2026-11-08) : retirer l'ancien timer de 14 h 10
systemctl --user disable --now edge-recommend.timer
rm -f ~/.config/systemd/user/edge-recommend.timer
```

## Alertes de panne

Depuis la phase 21, un service qui échoue se signale sur Discord au lieu de ne se voir que dans `journalctl`. Le message nomme l'unité,
l'heure et la commande à lancer pour lire la cause. Une même unité n'alerte qu'une fois toutes les six heures (table `alert_log`) : le
relevé de clôture passe toutes les dix minutes, une soirée sans réseau ne doit pas envoyer cinquante messages. Si la base elle-même ne
s'ouvre pas, l'alerte part quand même, sans mémoire.

```sh
edge notify-failure edge-snapshot.service            # essai à la main : un vrai message part (puis plus rien pendant six heures)
systemctl --user status 'edge-notify-failure@*'      # les alertes envoyées ou tentées
```

Le relevé du Loto Foot a en plus sa propre alerte, « source figée » : prono-match affiche encore comme grille en cours une grille close
depuis trois heures. Elle part une fois par grille ; la pastille « Loto Foot » de l'accueil et le bandeau de `/lotofoot` restent rouges
tant que la page n'est pas revenue (sources §73).

## Sauvegarde

`edge backup` copie la base (`VACUUM INTO`, instantané cohérent même si un timer écrit), vérifie la copie, puis ne garde que les
`backup_keep` plus récentes (7 par défaut). Les copies s'appellent `edge-AAAA-MM-JJ-HHMMSSZ.db`, en permissions 0600.

- `backup_dir` (edge.toml, chemin absolu) doit pointer **hors du disque du portable** : disque externe ou dossier synchronisé. edge
  n'envoie rien nulle part lui-même.
- Le dossier n'est jamais créé. S'il manque (disque non branché), la commande échoue et l'alerte de panne te le dit : mieux qu'une
  « sauvegarde » écrite sans bruit sur le disque interne.
- `edge backup --to /un/dossier` : une copie ponctuelle ailleurs, par exemple avant une manipulation.
- Restaurer : arrêter les timers et `edge-serve`, copier la sauvegarde à la place de `~/.local/share/edge/edge.db`, relancer.

## Grand Loto et EuroMillions

`edge-loto` relève chaque soir l'historique officiel de la FDJ (archives ZIP publiques), cherche l'alerte EuroMillions (quatrième tirage
consécutif au plafond sans gagnant : le cinquième verse le jackpot au rang 2), puis lance la séance d'un Grand Loto inscrit dans
`grand_loto_editions` (edge.toml) s'il a lieu dans les 3 jours. Une alerte n'est envoyée qu'une fois ; une séance n'est jamais refaite.

- `edge loto recommend --date 2026-12-25 --dry-run` : la séance d'une édition sans rien écrire.
- `edge loto played --last-session` : après validation, déclare les grilles jouées (ou `played 3 4`, ou `--grid 2-3-30-35-44+10 --date 2026-12-25`).
- `edge loto study` : rapport HTML (Grand Loto passés, préférences du public validées hors échantillon, EuroMillions au plafond, journal).

## Loto Foot

`edge-lotofoot-fetch` relève chaque heure la grille en cours du LF7 et du LF8 sur prono-match (répartition FDJ), et les derniers résultats
et rapports : la dérive de la répartition jusqu'à la clôture est ainsi mesurée, et chaque grille est réglée dès ses résultats publiés. Une
grille vue pour la première fois est signalée sur Discord avec son heure de clôture, sans ticket (il est trop tôt).
La séance se lance au guichet, juste avant de valider : bouton « Optimiser maintenant » de la page `/lotofoot` (ou `edge lotofoot recommend`).
Tickets d'un euro dont l'espérance, dans le scénario défavorable, atteint 1,7, dans la mise demandée et le budget de la semaine (16 €,
lundi-dimanche) ; grilles regroupées en multiples sans perte. Le timer du samedi midi (`edge-lotofoot-recommend`) est retiré depuis le
2026-10-24 : il calculait trop tôt et consommait le budget de la semaine. Pour le désinstaller :

```sh
systemctl --user disable --now edge-lotofoot-recommend.timer
rm -f ~/.config/systemd/user/edge-lotofoot-recommend.timer ~/.config/systemd/user/edge-lotofoot-recommend.service
systemctl --user daemon-reload
```

- `edge lotofoot recommend --dry-run` : la séance sans rien écrire ni relever (aucun crédit).
- `edge lotofoot played --last-session` : après validation chez le buraliste, déclare les tickets joués (ou `played 12 13`, ou
  `played --grid lf7:133 --ticket 1N22121` pour un ticket saisi à la main, hors modèle s'il ne correspond à aucune recommandation).
- `edge lotofoot report`, page `/lotofoot` de l'application : bilans et vérification du modèle (critère figé, points de contrôle à 26 et 52 semaines).
- Un nom d'équipe inconnu écarte sa grille : l'ajouter à la section `LOTOFOOT` de `config/aliases.toml`, puis réinstaller `edge`.
- Une rencontre absente de The Odds API (coupe nationale…) est cherchée chez OddsPapi si `oddspapi_api_key` est renseignée : 1 requête
  par séance, sous `oddspapi_monthly_cap`. Introuvable, elle écarte sa grille, avec des pistes pour écrire le nom OddsPapi dans
  `LOTOFOOT` (`docs/sources.md` §62).

## NBA

`edge-nba` archive chaque soir à 22 h UTC (18 h à New York en heure d'été : 1 à 4 h avant les matchs du soir américain, donc des cotes proches
de la clôture) les cotes de Pinnacle et des opérateurs français sur trois marchés (3 crédits), et un jour sur deux les résultats via `/scores`
(2 crédits, trois jours en arrière : un appel manqué ne perd rien). Hors saison (juillet à septembre), rien n'est dépensé. Le tennis est
refusé avant la NBA si le budget manque (`nba_reserve_credits`). Il n'existe aucun historique gratuit : les résultats s'accumulent depuis
l'ouverture (20 octobre 2026).

- `edge nba snapshot --dry-run` : le coût sans rien appeler.
- `edge nba report` : rapport HTML, **exploratoire** jusqu'au réglage (descriptif, ne valide rien).
- `edge nba tune` : une seule fois, **à partir du 1er février 2027** (la validation va du 15 novembre au 1er février) ; gèle le modèle.
  Le rapport évalue ensuite le test, à partir du 1er février.

## Tennis

Les cotes de tennis sont archivées par le relevé de 14 h (tournois en cours, 1 crédit chacun, 6 au plus ; voir `tennis_archive` et
`tennis_max_tournaments` dans la configuration). Les résultats et cotes historiques viennent des tableurs de tennis-data.co.uk,
**téléchargés à la main** (leurs conditions et leur `robots.txt` visent les robots) :

1. Page http://tennis-data.co.uk/data.php : télécharger les fichiers 2013 à aujourd'hui, ATP puis WTA.
2. Les ranger dans `~/.local/share/edge/tennis-data/atp/` et `~/.local/share/edge/tennis-data/wta/` (même nom `2025.xlsx` pour les
   deux circuits, d'où deux dossiers). Le fichier de l'année en cours se retélécharge de temps en temps.
3. `edge tennis import` puis `edge tennis check` ; `edge tennis tune` (une fois : règle l'Elo sur 2022-2023 et le gèle) ;
   `edge tennis report` (le rapport HTML : l'Elo contre le marché sur le test, à partir de 2024).

## Application Web (`edge serve`)

L'application d'edge, consultée depuis le PC de la maison : recommandations du jour, saisie des paris et des règlements,
bankroll, CLV et calibration, paper trading, bonus, Loto Foot, latence, rapports (quotidiens, archivés dans
`~/.local/share/edge/reports/daily/` et jamais écrasés, et d'analyse). Les pages sont recalculées à chaque visite. Elle écrit au
journal par les mêmes fonctions que la ligne de commande, mais n'appelle jamais l'API de cotes : elle ne dépense aucun crédit.

1. Dans `~/.config/edge/edge.toml` (permissions 0600), écouter sur le réseau, donner l'adresse que les autres appareils taperont
   (rappelée par Discord) et un secret de connexion, demandé une fois par navigateur :

   ```toml
   serve_bind = "0.0.0.0:8737"
   site_url = "http://cachyos-x8664.local:8737/"   # nom mDNS (avahi) ; sinon l'adresse IP du portable
   serve_token = "..."                              # sortie de `openssl rand -hex 24`
   ```

2. Ouvrir le port dans `ufw` (actif sur ce portable), pour le réseau local seulement. Adapte le sous-réseau au tien (`ip -4 addr`) :

   ```sh
   sudo ufw allow from 192.168.1.0/24 to any port 8737 proto tcp
   ```

3. Installer et démarrer le service :

   ```sh
   cp deploy/systemd/edge-serve.service ~/.config/systemd/user/
   systemctl --user daemon-reload
   systemctl --user enable --now edge-serve.service
   ```

Après chaque `cargo install`, `systemctl --user restart edge-serve.service` : le service garde l'ancien binaire en mémoire.
Le site est en HTTP : le secret et le cookie de session circulent en clair, ce qui suffit sur le réseau de la maison, pas au-delà.
Il ne répond que si le portable est allumé.

L'ancienne application egui (`edge-gui`, phases 6 et 9) est retirée. Pour effacer ce qu'elle avait installé :

```sh
rm -f ~/.local/bin/edge-gui ~/.local/share/applications/edge-gui.desktop
```

## Webhook Discord

La notification a besoin de l'URL d'un webhook (Discord : réglages du salon → Intégrations → Webhooks → Nouveau webhook →
Copier l'URL). Cette URL est un secret : elle donne le droit d'écrire dans le salon. Ajoute-la dans
`~/.config/edge/edge.toml` (permissions 0600, déjà celles du fichier créé par `edge init`) :

```toml
discord_webhook_url = "https://discord.com/api/webhooks/..."
```

La variable `EDGE_DISCORD_WEBHOOK` a priorité sur le fichier. Ne la colle ni dans le chat ni dans git.

`edge notify-test` envoie un message de test avec une image d'exemple (recommandation fictive) : c'est le rendu de l'image que joint
le message du jour quand il y a des recommandations.

## Surveiller

```sh
systemctl --user list-timers 'edge-*'                 # prochains déclenchements
journalctl --user -u edge-snapshot.service -n 50      # dernier relevé
journalctl --user -u edge-recommend.service -n 50     # dernières recommandations
journalctl --user -u edge-closing.service -n 50       # dernières clôtures
journalctl --user -u edge-serve.service -n 50         # dernières pages servies
edge quota                                            # crédits consommés (gratuit)
edge clv                                              # CLV réel des recommandations et des paris
```

Lancer à la main : `edge snapshot` (6 crédits, ou `--dry-run` pour le coût), `edge recommend` (sans `--notify` : écrit le journal,
n'envoie rien), `edge closing --dry-run` (liste les matchs concernés). Un relevé manqué ne se rattrape pas : l'historique
des cotes est payant et hors budget.

**Après toute phase qui ajoute une migration, réinstaller le binaire** (`cargo install …` ci-dessus). Sinon les unités échouent
avec « la base est en version N mais ce programme ne connaît que la version N−1 » : la base de production est migrée dès la
première commande lancée avec le nouveau binaire de développement.
