#!/usr/bin/env python3
"""Génère crates/core/tests/fixtures/penaltyblog_reference.json : un petit championnat simulé et l'ajustement
Dixon-Coles de `penaltyblog` sur ces données, servant de référence au test Rust (`models/reference.rs`).

Utilisation (environnement jetable, rien n'est installé hors du dossier indiqué) :

    python3 -m venv /tmp/edge-ref && /tmp/edge-ref/bin/pip install penaltyblog
    /tmp/edge-ref/bin/python scripts/penaltyblog_reference.py

Le fichier produit est versionné : le test Rust n'a besoin ni de Python ni de `penaltyblog`.
"""
import json
import pathlib

import numpy as np
import pandas as pd
import penaltyblog as pb

XI = 0.0018  # décroissance temporelle par jour (valeur par défaut de penaltyblog)
TEAMS = ["A", "B", "C", "D", "E", "F", "G", "H"]
ATTACK = np.array([0.35, 0.2, 0.1, 0.0, -0.05, -0.15, -0.2, -0.25])
DEFENCE = np.array([-0.3, -0.15, -0.05, 0.0, 0.05, 0.1, 0.2, 0.25])
BASE, HOME_ADVANTAGE = 0.1, 0.3

rng = np.random.default_rng(2026)
rows = []
day = 0
for _ in range(6):  # six aller-retour complets = 336 matchs
    for h in range(8):
        for a in range(8):
            if h != a:
                lam = np.exp(BASE + HOME_ADVANTAGE + ATTACK[h] + DEFENCE[a])
                mu = np.exp(BASE + ATTACK[a] + DEFENCE[h])
                rows.append((h, a, int(rng.poisson(lam)), int(rng.poisson(mu)), day))
                day += 1  # un match par jour : les poids temporels varient d'un match à l'autre

df = pd.DataFrame(rows, columns=["home", "away", "goals_home", "goals_away", "day"])
dates = pd.Timestamp("2024-08-01") + pd.to_timedelta(df["day"], unit="D")
weights = pb.models.dixon_coles_weights(dates, XI)

model = pb.models.DixonColesGoalModel(
    df["goals_home"], df["goals_away"],
    df["home"].map(lambda i: TEAMS[i]), df["away"].map(lambda i: TEAMS[i]),
    weights,
)
model.fit()
params = model.get_params()
hfa, rho = float(params["home_advantage"]), float(params["rho"])

pairs = []
for h in range(8):
    for a in range(8):
        if h != a:
            lam = float(np.exp(hfa + params[f"attack_{TEAMS[h]}"] + params[f"defence_{TEAMS[a]}"]))
            mu = float(np.exp(params[f"attack_{TEAMS[a]}"] + params[f"defence_{TEAMS[h]}"]))
            probs = model.predict(TEAMS[h], TEAMS[a]).home_draw_away
            pairs.append({"home": h, "away": a, "lambda": lam, "mu": mu,
                          "p_home": float(probs[0]), "p_draw": float(probs[1]), "p_away": float(probs[2])})

out = {
    "penaltyblog_version": pb.__version__,
    "xi": XI,
    "games": df.to_dict(orient="records"),
    "home_advantage": hfa,
    "rho": rho,
    "attack": [float(params[f"attack_{t}"]) for t in TEAMS],
    "defence": [float(params[f"defence_{t}"]) for t in TEAMS],
    "pairs": pairs,
}
target = pathlib.Path(__file__).resolve().parent.parent / "crates/core/tests/fixtures/penaltyblog_reference.json"
target.write_text(json.dumps(out, indent=1))
print(f"{len(df)} matchs, rho={rho:.4f}, avantage={hfa:.4f} -> {target}")
