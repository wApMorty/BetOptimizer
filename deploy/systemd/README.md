# Planification (systemd utilisateur)

Prérequis : lingering activé (`loginctl enable-linger $USER`) et binaire installé dans `~/.local/bin`.

| Timer | Quand | Ce qu'il fait | Crédits |
|---|---|---|---|
| `edge-snapshot` | 14 h (Paris) | relevé des cotes de 6 ligues (`edge snapshot`) | 6 par jour |
| `edge-recommend` | 14 h 10 | recommandations du jour et notification Discord (`edge recommend --notify`) | 0 |
| `edge-nba` | 22 h UTC | cotes de la NBA (vainqueur, handicaps, totaux) et, un jour sur deux, résultats (`edge nba snapshot`) | 3 par jour + 2 tous les deux jours |
| `edge-closing` | toutes les 10 minutes | clôture de Pinnacle des matchs suivis qui commencent dans 30 minutes (`edge closing`) | 1 par match suivi, 0 sinon |

```sh
# Installer ou mettre à jour le binaire
cargo install --path crates/cli --locked --root ~/.local

# Installer et activer les timers
mkdir -p ~/.config/systemd/user
cp deploy/systemd/edge-*.service deploy/systemd/edge-*.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now edge-snapshot.timer edge-recommend.timer edge-closing.timer edge-nba.timer
```

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

## Application (`edge-gui`)

```sh
cargo install --path crates/gui --locked --root ~/.local     # compile en release : environ 3 minutes la première fois
mkdir -p ~/.local/share/applications
sed "s|^Exec=.*|Exec=$HOME/.local/bin/edge-gui|" deploy/edge-gui.desktop > ~/.local/share/applications/edge-gui.desktop
```

L'application apparaît ensuite dans le lanceur d'applications (« edge »), ou se lance par `edge-gui`. Elle lit et écrit le journal
(recommandations, paris, règlements) mais n'appelle jamais l'API de cotes : elle ne dépense aucun crédit.

## Webhook Discord

La notification a besoin de l'URL d'un webhook (Discord : réglages du salon → Intégrations → Webhooks → Nouveau webhook →
Copier l'URL). Cette URL est un secret : elle donne le droit d'écrire dans le salon. Ajoute-la dans
`~/.config/edge/edge.toml` (permissions 0600, déjà celles du fichier créé par `edge init`) :

```toml
discord_webhook_url = "https://discord.com/api/webhooks/..."
```

La variable `EDGE_DISCORD_WEBHOOK` a priorité sur le fichier. Ne la colle ni dans le chat ni dans git.

## Surveiller

```sh
systemctl --user list-timers 'edge-*'                 # prochains déclenchements
journalctl --user -u edge-snapshot.service -n 50      # dernier relevé
journalctl --user -u edge-recommend.service -n 50     # dernières recommandations
journalctl --user -u edge-closing.service -n 50       # dernières clôtures
edge quota                                            # crédits consommés (gratuit)
edge clv                                              # CLV réel des recommandations et des paris
```

Lancer à la main : `edge snapshot` (6 crédits, ou `--dry-run` pour le coût), `edge recommend` (sans `--notify` : écrit le journal,
n'envoie rien), `edge closing --dry-run` (liste les matchs concernés). Un relevé manqué ne se rattrape pas : l'historique
des cotes est payant et hors budget.

**Après toute phase qui ajoute une migration, réinstaller le binaire** (`cargo install …` ci-dessus). Sinon les unités échouent
avec « la base est en version N mais ce programme ne connaît que la version N−1 » : la base de production est migrée dès la
première commande lancée avec le nouveau binaire de développement.
