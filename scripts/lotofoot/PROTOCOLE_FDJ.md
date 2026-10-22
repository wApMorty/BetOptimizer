# Backtest Loto Foot sur les copies Wayback des grilles FDJ — protocole figé le 2026-10-22, avant lecture

Écrit pendant la collecte, avant d'avoir lu un seul résultat de grille au-delà de l'exemple de structure (LF7 n°4583, août 2024).

## Données
- Grilles LF7, LF8, LF12, LF15 du site des points de vente FDJ, dernière copie Wayback de chaque page ; état JSON `gridResolver`.
- Retenues : `drawCancelled` faux, un résultat pour chaque match, `winranks` présents, mêmes totaux de `numberPlayed` sur tous les matchs.
- q : `numberPlayed` / S, S = total de `numberPlayed` d'un match (mises en euros). **Approximation assumée** : q et S finaux (à la
  clôture) au lieu de la répartition visible une heure avant ; l'écart sera mesuré par le relevé prospectif de prono-match.
- p : cotes d'avant-match football-data (PSH, sinon AvgH, sinon B365H), méthode puissance. Amendement du 2026-10-22, avant
  lecture : les fichiers « new » (Suisse, Autriche…) n'ont que la clôture (PSCH, sinon AvgCH, sinon B365CH) ; elle est utilisée
  faute de mieux (fuite légère : informations entre la clôture de la grille et le coup d'envoi). Sensibilité annoncée : mêmes
  mesures sur les seules grilles sans aucune cote de clôture. Analyse principale : seulement les grilles dont tous les matchs ont une cote. Analyse secondaire annoncée : matchs sans
  cote remplacés par la foule débiaisée p̂ ∝ q^(1/γ) (γ estimé sur l'entraînement).
- Masses : 70 % de S ; LF7 et LF8 45/55, LF12 et LF15 40/20/20/20. Rang 1 : max(part naturelle, montant garanti annoncé dans
  `jackpot` s'il existe). Masse effective d'un rang quand nous gagnons : la masse calculée (pas le gain publié, qui ignore notre ticket).

## Découpage temporel (aucune fuite)
- Entraînement : grilles closes avant le 2024-01-01. On y estime γ (public débiaisé) et le modèle des concurrents
  W ~ Poisson(κ λ^β + plancher), par maximum de vraisemblance sur tous les rangs (λ calculé avec le vrai q). Amendement du 2026-10-22, sur 346 grilles LF7
  d'entraînement et avant toute donnée de test : vraisemblance binomiale négative (dispersion θ estimée), car la loi de Poisson,
  dominée par les rangs à des milliers de gagnants, sous-prédisait les faibles λ (rapport 1,38 entre 10 et 100). Second
  amendement, même moment : le plancher fixe devient φ · S / 3^n (part φ des mises répartie sur toutes les combinaisons), seule
  forme cohérente entre LF7 (2 187 combinaisons) et LF15 (14 millions). Concurrents : κ λ^β + φ · S / 3^n.
- Test : grilles closes à partir du 2024-01-01, évaluées une seule fois avec les paramètres gelés.

## Stratégie (une seule configuration principale)
- 10 tickets simples par grille, distincts d'au moins 2 signes, choisis glouton sur l'espérance du modèle gelé.
- Principale : toutes les grilles du test. Secondaire annoncée : seulement les grilles où l'espérance moyenne des 10 tickets dépasse 1,2.
- Gain réalisé : au rang atteint, masse du rang / (W publiés + nos tickets à ce rang) ; mise 1 € par ticket.

## Mesures
- ROI réalisé, IC 95 % par bootstrap des semaines ISO (10 000 tirages, graine 20261022), par jeu et au total.
- Espérance annoncée par le modèle sur les mêmes grilles (pour comparer annoncé et réalisé).
- Calibration hors échantillon : rapport W publiés / prédits sur le test, par rang et par tranche de λ.
- Configurations : 1 principale + 1 secondaire (seuil 1,2) + l'analyse « foule débiaisée ». Rien d'autre ne sera essayé sur le test.

## Amendement du 2026-10-22 à 18 h 20 (collecte LF12/LF15 en cours, aucun résultat de test lu)
- À la demande de Paul, LF7 et LF8 sont évalués d'abord. Leurs paramètres (κ, β, φ, θ et γ de la foule débiaisée) sont
  ajustés sur leur seul entraînement (LF7 + LF8, grilles closes avant 2024) et **gelés définitivement** pour ces deux jeux :
  l'évaluation de LF7/LF8 qui suit est l'unique évaluation de leur test.
- LF12 et LF15 auront leur propre ajustement (LF12 + LF15, avant 2024) à la fin de la collecte, puis une évaluation unique.
