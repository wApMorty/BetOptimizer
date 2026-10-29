# Backtest Loto Rugby 7 — protocole figé avant toute lecture du test (rédigé le 2026-10-29, à valider par Paul)

État des lectures au moment de la rédaction :
- **entraînement** (grilles closes avant le 2024-01-01) : retour du public, mises S et calibration de la répartition par tranche
  (`payout.py`, `public_calib.py`, §61) ;
- **test** (grilles closes à partir du 2024-01-01) : seulement le nombre de grilles, leurs dates, le nombre de matchs et les montants
  garantis ; **aucun résultat, aucun rapport, aucune répartition**. Le tableau `payout.py` du §61 inclut 2024-2026 (retour du public
  par saison, 83 à 85 %) : c'est une lecture du mécanisme de garantie, pas des tickets ; elle est déclarée ici.

## Données
- Grilles Loto Rugby 7 du site des points de vente FDJ : copies Wayback (`wb_get.py`) et relevé direct (`fdj_get.py`), faits extraits.
- Retenues : non annulées, 7 matchs, un résultat 1 / N / 2 pour chaque match, deux rangs publiés avec au moins un gagnant chacun,
  même composition (équipes, ordre) dans la copie qui porte la répartition et dans celle qui porte les résultats.
  Disponibles : **77 grilles d'entraînement, 60 de test**.
- q : copie la plus tardive dont les pourcentages somment à au moins 95 sur chaque match. Ces copies sont prises après la clôture
  (l'ancien site gardait la répartition) : **q est la répartition finale**, comme au Loto Foot (§37). Un 0 % devient 0,25 %, puis
  chaque match est renormalisé à 1.
- S réel : mises déduites du 2e rang, S = W₂ · g₂ / 0,385 (`payout.py`).

## Découpage temporel (aucune fuite)
- Entraînement : grilles closes avant le 2024-01-01. Tous les paramètres y sont estimés, puis écrits dans `params_rugby.json`
  (empreinte SHA-256 consignée dans `docs/sources.md` avant le test).
- Test : grilles closes à partir du 2024-01-01, évaluées **une seule fois** avec ces paramètres gelés.

## Modèle des probabilités p (principal, un seul)
Le handicap de la FDJ équilibre les matchs (§61) ; le modèle n'utilise donc ni cote ni répartition :
- p(N) = π_N, fréquence des « N » à l'entraînement ;
- p(1) = (1 − π_N) · h et p(2) = (1 − π_N) · (1 − h), h = part des « 1 » parmi les matchs sans « N » à l'entraînement ;
- matchs indépendants : P(issue o) = Πᵢ p(oᵢ).

## Modèle des concurrents (repris du Loto Foot, §37 et `PROTOCOLE_FDJ.md`)
Pour une issue o et un rang r (7 bons, puis 6 bons), λ_r(o) = S · P_q(r bons | o), loi de Poisson-binomiale des grilles du public
tirées signe par signe selon q (Clair & Letscher, *Operations Research*, 2007).
Concurrents W ~ binomiale négative de moyenne μ = κ · λ^β + φ · S / 3⁷ et de dispersion θ. κ, β, φ, θ sont estimés par maximum de
vraisemblance sur les deux rangs des grilles d'entraînement (λ calculé avec la vraie issue).

## Masses et gains (règlement Loto Sports v6, art. 7.6, 10.1.1, 10.1.5, 10.1.7.5)
- Rang 1 : M₁ = max(0,315 · S, montant garanti `jackpot` de la page) ; rang 2 : M₂ = 0,385 · S.
- Espérance d'un ticket c (mise 1 €), S remplacé par Ŝ, avec E[1/(1+W)] calculé sur la loi binomiale négative :

      EV(c) = P(c) · M₁ · E[1/(1 + W₁(c))] + Σ_{o à une erreur de c} P(o) · M₂ · E[1/(1 + W₂(o))]

- **Ŝ ex ante** : médiane de S réel des 8 dernières grilles Loto Rugby 7 closes avant l'ouverture de la grille (médiane de
  l'entraînement s'il y en a moins de 8). Le relevé ne voit que des pourcentages : en production aussi, S est inconnu au moment du choix.

## Stratégie (une seule configuration principale)
- **10 tickets simples distincts par grille** (un signe d'écart suffit), choisis par ordre décroissant d'EV, toutes les grilles du
  test, sans seuil.
- Gain réalisé, mise 1 € par ticket, S réel + 10 € : au rang atteint, masse du rang / (gagnants publiés + nos tickets à ce rang).
  Rang 1 arrondi à l'euro inférieur, rang 2 au dixième inférieur ; si le gain unitaire du rang 1 tombe sous celui du rang 2, les deux
  masses sont réunies et partagées entre tous leurs gagnants (art. 10.1.1).

## Mesures
- ROI réalisé et IC 95 % par bootstrap des grilles (10 000 tirages, graine 20261029). ROI sans le meilleur gain. Nombre de grilles
  payées, observé et attendu.
- Espérance annoncée par le modèle sur les mêmes tickets.
- Calibration hors échantillon : rapport des gagnants publiés aux gagnants prédits sur le test, par rang et par tranche de λ.
- Loi simulée du gain sous le modèle : 20 000 tirages des issues selon p (concurrents à leur loi binomiale négative).
- Hypothèse nulle « tickets du public » : 10 000 jeux de 10 tickets tirés signe par signe selon q, gains calculés avec les gagnants
  publiés ; P = part des jeux qui font au moins le gain réalisé.

## Verdict (règles fixées d'avance)
- **Modèle en défaut** si une tranche λ ≥ 10 d'au moins 10 observations a un rapport hors de [0,8 ; 1,25], ou si le gain réalisé tombe
  sous le 5e centile de sa loi simulée.
- **Avantage démontré** : borne basse de l'IC 95 % du ROI au-dessus de 0.
- **Avantage probable** : ROI positif, P < 0,05 contre les tickets du public, et modèle pas en défaut.
- **Non démontré** dans tous les autres cas.

## Analyses secondaires annoncées (rapportées, sans rôle dans le verdict)
1. Modèle logistique : p(côté) = σ(a + b · logit q_côté), entre 1 et 2 hors « N », a et b estimés sur l'entraînement.
2. 1 et 3 tickets par grille au lieu de 10.
3. S réel au lieu de Ŝ dans le choix des tickets.

Configurations : 1 principale + 4 secondaires. Rien d'autre ne sera essayé sur le test.

## Ordre d'exécution après validation
1. `rugby_fit.py` : estimation sur l'entraînement, écriture de `params_rugby.json`, ROI de l'entraînement (dans l'échantillon,
   déclaré comme tel), empreinte consignée et committée.
2. `rugby_test.py` : lecture unique du test, rapport complet, consigné dans `docs/sources.md`, quel que soit le résultat.
