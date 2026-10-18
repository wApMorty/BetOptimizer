# Planification (systemd utilisateur)

Prérequis : lingering activé (`loginctl enable-linger $USER`) et binaire installé dans `~/.local/bin`.

| Timer | Quand | Ce qu'il fait | Crédits |
|---|---|---|---|
| `edge-snapshot` | 14 h (Paris) | relevé des cotes de 6 ligues (`edge snapshot`) | 6 par jour |
| `edge-recommend` | 14 h 10 | recommandations du jour et notification Discord (`edge recommend --notify`) | 0 |
| `edge-closing` | toutes les 10 minutes | clôture de Pinnacle des matchs suivis qui commencent dans 30 minutes (`edge closing`) | 1 par match suivi, 0 sinon |

```sh
# Installer ou mettre à jour le binaire
cargo install --path crates/cli --locked --root ~/.local

# Installer et activer les timers
mkdir -p ~/.config/systemd/user
cp deploy/systemd/edge-*.service deploy/systemd/edge-*.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now edge-snapshot.timer edge-recommend.timer edge-closing.timer
```

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
