# Bonus de bienvenue : stratégie et suivi

> Document de travail ouvert le 2026-10-20, à ajuster à chaque ouverture de compte.
> Les mesures viennent de scripts ponctuels, en lecture seule sur la base ; le suivi et les calculs sont outillés depuis la
> phase 12 (§8).

## 1. Pourquoi cette piste

Aucune stratégie de pari mesurée jusqu'ici ne bat le marché de façon démontrable (`docs/sources.md` §16, §18, §27, §30).
Un bonus de bienvenue, au contraire, a une **espérance positive par construction** : l'opérateur rend tout ou partie de la
mise perdue. Le gain est unique (une fois par opérateur), mais il est grand devant les 15 à 30 € par mois des cotes boostées.

## 2. Règle d'éligibilité (à vérifier opérateur par opérateur)

- Winamax, règlement du 18 avril 2025, clause (C) : l'offre est valable « une seule fois par utilisateur, terminal
  (ordinateur, tablette ou mobile), foyer, carte bancaire […] ou compte bancaire » et « réservée aux joueurs n'ayant jamais
  effectué de dépôt sur un compte Winamax (y compris sur un compte qui a été fermé depuis) ». **Winamax est donc exclu.**
- Paul a touché une seconde fois des offres après une pause de 2019 à 2022 : les règles ont pu changer depuis, ou c'étaient
  d'autres opérateurs, ou des offres de retour pour joueurs inactifs.
- **Avant tout dépôt** : lire la clause d'éligibilité du règlement en vigueur, ou poser la question au service client. Noter
  la réponse et la date avec l'offre (`edge bonus offer --source …`, §8).
- Les offres de retour pour joueurs inactifs (courriel, application) sont permises : à évaluer avec les mêmes formules.

## 3. Valeur d'une offre selon sa forme

Notations : `S` la mise, `o` la cote obtenue, `p` la probabilité réelle de l'issue (Pinnacle sans marge, méthode
multiplicative), `r = p · o` le retour du pari (1 = prix juste ; 0,90 = 10 % de marge). Calcul élémentaire de l'espérance.

| Forme de l'offre | Gain si l'issue passe | Gain si elle perd | Espérance |
|---|---|---|---|
| Freebet `F` (la mise n'est pas rendue) | `F · (o − 1)` | 0 | `F · (r − p)` |
| Premier pari remboursé **en argent** | `S · (o − 1)` | 0 (mise rendue) | `S · (r − p)` |
| Premier pari remboursé **en freebet** de valeur `v` par euro | `S · (o − 1)` | `−S + v · S` | `S · (r − 1) + (1 − p) · v · S` |
| Freebet sans dépôt | comme un freebet | | `F · (r − p)` |

Les deux premières formes ne font **jamais perdre d'argent** : le seul risque est de ne rien gagner. L'espérance `r − p`
croît avec la cote (p diminue) tant que la marge sur les outsiders n'augmente pas trop.

**Mesure sur l'archive** (football, 3 jours de relevés, 2026-10-18 au 2026-10-20 ; observations corrélées, ordre de
grandeur seulement). Valeur d'un freebet de 1 € joué sans couverture, `r − p` :

| Cote française | PMU | Unibet | Betclic | NetBet |
|---|---|---|---|---|
| 2,0–3,0 | 0,52 | 0,52 | 0,52 | 0,53 |
| 3,0–4,5 | 0,64 (0,68*) | 0,64 (0,69*) | 0,65 (0,69*) | 0,64 (0,67*) |
| 4,5–6,5 | 0,70 (0,77*) | 0,70 (0,77*) | 0,71 (0,79*) | 0,71 (0,75*) |
| 6,5–10 | 0,73 (0,84*) | 0,73 (0,92*) | 0,74 (0,89*) | 0,73 (0,78*) |

\* moyenne sur les 10 % de prix les plus proches de Pinnacle : choisir son pari rapporte de 5 à 15 points de conversion.
Les tranches au-delà de 6,5 comptent de 13 à 26 observations : chiffres fragiles.

Les « 65 à 80 % » des forums sont l'espérance sans couverture, pas un gain garanti.

## 4. Couverture : gain garanti

Couvrir, c'est miser de l'argent chez d'autres opérateurs sur les issues contraires, de sorte que le gain soit le même quel
que soit le résultat. Formules (calcul élémentaire, en égalisant le gain sur les trois issues ; `o_i` est la meilleure cote
de l'issue `i` chez les opérateurs de couverture, `X` l'issue jouée avec le bonus) :

| Ce qu'on couvre | Mise de couverture sur chaque autre issue `i` | Gain garanti |
|---|---|---|
| Freebet `F` à la cote `o` | `c_i = F · (o − 1) / o_i` | `F · (o − 1) · (1 − Σ 1/o_i)` |
| Premier pari `S` remboursé **en argent** | `c_i = S · (o − 1) / o_i` | `S · (o − 1) · (1 − Σ 1/o_i)` |
| Premier pari `S` remboursé **en freebet**, ce freebet étant lui-même couvert pour `g` par euro | `c_i = S · (o − g) / o_i` | `S · [(o − 1) − (o − g) · Σ 1/o_i]` |

**Mesure sur l'archive** (football, 3 jours de relevés, meilleur match de chaque jour ; bonus chez PMU, Unibet, Betclic ou
NetBet, couverture chez les quatre autres opérateurs suivis, Winamax compris) :

| Opération | Gain garanti | Cote jouée | Capital de couverture immobilisé |
|---|---|---|---|
| Freebet couvert (`g`) | 0,53 à 0,56 par euro (0,50 à 0,54 si la couverture n'est que chez Winamax) | 3,4 à 5 | environ 2,5 fois le freebet |
| Premier pari remboursé en argent, couvert | 0,53 à 0,56 par euro (même formule) | 3,4 à 5 | environ 2,5 fois la mise |
| Premier pari remboursé en freebet, couvert, avec `g` = 0,55 | 0,17 à 0,22 par euro | 2,5 à 3,7 | 1,3 à 2,5 fois la mise |

Seules 12 % des lignes donnent `g ≥ 0,50` : il faut choisir le match, d'où l'intérêt d'un calcul sur le relevé du jour.

## 5. Deux modes, à choisir offre par offre

| Portefeuille type : 1 remboursement en argent (100 €), 7 remboursements en freebet (100 €), 2 freebets sans dépôt (10 €) | Gain | Risque |
|---|---|---|
| **Tout couvert** | environ **+200 € garantis** (55 + 7 × 20 + 2 × 5,5) | aucun, hors erreur d'exécution (§6) |
| Rien couvert, cote 5 (simulation, 200 000 tirages, graine 1) | +477 € en espérance, médiane +500 € | 21 % de finir en perte, pire cas −700 € |

Paul veut être sûr de récupérer l'argent avancé : **mode retenu, tout couvert**. Le mode sans couverture rapporte environ
deux fois plus en espérance, au prix d'une chance sur cinq de perdre.

## 6. Règles d'exécution

1. **Comptes de couverture.** Couvrir uniquement chez un opérateur dont le bonus est déjà consommé (ou impossible : Winamax).
   Une couverture placée chez un opérateur neuf deviendrait son « premier pari » et gâcherait son offre. Première opération :
   couverture chez Winamax seul. Chaque opérateur utilisé rejoint ensuite les comptes de couverture.
2. **Ordre de placement.** Placer d'abord le pari bonus, puis recalculer les mises de couverture avec les cotes réellement
   affichées, et les placer aussitôt. Une cote qui bouge entre les deux paris change le gain de quelques euros : vérifier
   avant chaque validation.
3. **Arrondis.** Arrondir les mises de couverture au 0,10 € le plus proche ; l'écart de gain entre issues reste de quelques
   centimes.
4. **Capital.** Prévoir environ 250 € de couverture par opération, en plus de la mise ; il revient après le match.
5. **Choix du match.** 1X2 des grandes ligues, cote du pari bonus entre 2,5 et 5 selon le §4, au match qui maximise le gain
   garanti. Respecter la cote minimale imposée par l'offre. **Pas de grosse cote quand on couvre** (vérifié le 2026-10-21,
   Betclic, pour 100 €) : premier pari remboursé en freebet, meilleur gain garanti 22 € et 243 € sortis à la cote 2–3, mais
   7 € et 668 € sortis à 6,5–10, −21 € au-delà de 10. La couverture porte alors sur le favori, avec de grosses mises payées
   à la marge française. Une grosse cote ne se justifie que sans couverture (§3).
6. **Match reporté ou annulé.** Les règles diffèrent selon les opérateurs (freebet rendu ou non, paris remboursés) : lire le
   règlement avant de choisir un match à risque de report.
7. Lire pour chaque offre : cote minimale, marchés éligibles, délai pour le premier pari, durée de validité et découpage des
   freebets, conditions de mise (« wagering ») éventuelles. Une condition de mise de `k` fois un montant `B` coûte environ
   `k · B · (1 − r)` : à déduire de la valeur.

## 7. Opérateurs agréés (liste ANJ lue le 2026-10-20)

Offres relevées sur des comparateurs (cotesports.fr, eurosport.fr, octobre 2026) : **non vérifiées**, à confirmer sur le
règlement de l'opérateur au moment de l'ouverture.

| Opérateur | Offre annoncée (comparateur) | Forme | Prix dans notre archive |
|---|---|---|---|
| Winamax | 100 € remboursés | argent | oui — **exclu** (dépôt passé) |
| PMU | 100 € remboursés | argent | oui |
| Betclic | 100 € remboursés | freebets | oui |
| Unibet (et ZEturf, même société) | premier pari doublé jusqu'à 110 € | à préciser | oui |
| NetBet | à relever | | oui (écarté des recommandations) |
| bet365 (agréé depuis mai 2026) | jusqu'à 100 € remboursés | à préciser | non |
| Betsson | 10 € sans dépôt + 100 € de freebets | freebets | non |
| bwin | à relever | | non |
| PokerStars Sports (et Betstars) | à relever | | non |
| Genybet | à relever | | non |
| Vbet | à relever | | non |
| Feelingbet | à relever | | non |
| Olybet | à relever | | non |
| Circus | à relever | | non |
| DAZN Bet | à relever | | non |
| Yesorno | à relever | | non |

La liste ANJ lue ne mentionnait ni Parions Sport (FDJ) ni ZEbet : à revérifier avant de les écarter.

## 8. Suivi : `edge bonus`, onglet Bonus, page `/bonus` (phase 12)

Le suivi est tenu par l'outil, dans un journal immuable : une offre consommée n'est plus jamais proposée, et son opérateur devient un
compte de couverture.

1. **Après lecture du règlement**, enregistrer l'offre (onglet Bonus, « Enregistrer l'offre », ou en ligne de commande) :
   `edge bonus offer --bookmaker betclic_fr --kind freebet_refund --amount 100 --source "règlement du …, article …"`.
   Pour un compte qui ne sert qu'à couvrir : `edge bonus offer --bookmaker winamax_fr --ineligible --source "règlement du 18/04/2025, clause C"`.
2. **Choisir l'opération** : `edge bonus status` (ou `pick --bookmaker …`), l'onglet Bonus, ou la page `/bonus` du site sur le téléphone.
3. **Au moment de parier**, placer le pari bonus, puis recalculer les couvertures sur les cotes affichées (tiroir « J'ai joué »,
   calculateur de l'onglet, ou `edge bonus cover`) et les placer aussitôt.
4. **Enregistrer** l'opération (tiroir « J'ai joué », ou la commande `edge bonus log` affichée par `status`).
5. **Régler** après le match : automatiquement depuis football-data (`edge bonus settle`), ou à la main. Un freebet reçu apparaît alors
   « à jouer » chez l'opérateur, avec ses propres propositions.

Valeur d'un euro de freebet supposée pour le premier pari remboursé en freebet : `bonus_freebet_value = 0.53` dans la configuration.

## 9. À ajuster

- Valeur d'un freebet couvert (`bonus_freebet_value`, 0,53) : la remplacer par la valeur réellement obtenue quand quelques freebets
  auront été joués.
- Tableau du §3 : refaire la mesure quand l'archive comptera quelques mois.

Sources : [ANJ, opérateurs agréés](https://anj.fr/offre-de-jeu-et-marche/operateurs-agrees) ;
[Winamax, règlement de l'offre de bienvenue 2025](https://operator-front-static-cdn.winamax.fr/img/content/betting/2025/20250418_Welcome_Bonus/RglementWB2025.pdf) ;
[cotesports.fr](https://cotesports.fr/bonus-paris-sportifs/) ; [eurosport.fr](https://www.eurosport.fr/paris-sportifs/bonus/) ;
[forum Coteur, conversion des freebets](https://forum.coteur.com/viewtopic.php?t=7402).
