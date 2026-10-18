# Relevé quotidien des cotes (systemd utilisateur)

Prérequis : lingering activé (`loginctl enable-linger $USER`) et binaire installé dans `~/.local/bin`.

```sh
# Installer ou mettre à jour le binaire
cargo install --path crates/cli --locked --root ~/.local

# Installer et activer le timer (14h, heure de Paris)
mkdir -p ~/.config/systemd/user
cp deploy/systemd/edge-snapshot.service deploy/systemd/edge-snapshot.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now edge-snapshot.timer
```

Surveiller :

```sh
systemctl --user list-timers edge-snapshot.timer     # prochain déclenchement
journalctl --user -u edge-snapshot.service -n 50     # dernier relevé
edge quota                                           # crédits consommés (gratuit)
```

Lancer un relevé à la main : `edge snapshot` (6 crédits). Le coût sans rien appeler : `edge snapshot --dry-run`.
Un relevé manqué ne se rattrape pas : l'historique des cotes est payant et hors budget.
