# Dérive de la répartition Loto Foot avant la clôture — protocole figé le 2026-10-24, avant toute donnée utile

Écrit avant l'analyse (accord de Paul du 2026-10-24) : 3 grilles relevées à cette date, regardées seulement pour la faisabilité
(`docs/sources.md` §50). Tout changement après cette date est un amendement daté, signalé comme tel dans le rapport.

## Question
1. De combien la répartition du public bouge-t-elle entre un relevé et la clôture, selon l'heure avant la clôture et l'âge de la grille ?
2. De combien l'espérance S3 annoncée d'un ticket choisi à ce moment dépasse-t-elle sa valeur avec la répartition de la clôture ?
3. À partir de quand une grille peut-elle engager du budget sans espérance gonflée, et quelles espérances signaler comme aberrantes ?

## Données
- Relevé horaire de prono-match (`edge lotofoot fetch`, timer `edge-lotofoot-fetch`, minute 7) : tables `lotofoot_grid`,
  `lotofoot_split`, `lotofoot_fetch`, `lotofoot_alert`. Le site publie la répartition FDJ une fois par heure, vers la minute 55
  (`split_at`, vérifié le 2026-10-24) : un relevé plus fréquent n'apporterait rien.
- Horodatage d'un relevé : `split_at` (heure annoncée par le site), pas l'heure du relevé. Plusieurs relevés au même `split_at` comptent
  pour un seul.
- Cotes : archive Pinnacle de `edge` (lecture identique à celle de la séance, `lotofoot::odds::archived_market`).
- Aucun résultat de match n'est lu : tout est ex ante.

## Grilles retenues
- LF7 et LF8 closes, vues pour la première fois par le relevé horaire alors que le relevé précédent, au plus 2 h plus tôt, ne les
  voyait pas (ouverture observée) ; âge `a` d'un relevé = heures depuis le premier `split_at` de la grille.
- Dernier relevé au plus 75 minutes avant la clôture ; aucun trou de plus de 3 h dans la série (sinon la grille est écartée, avec la
  raison, et comptée dans le rapport).
- Répartition de référence `q_ref` : le dernier relevé avant la clôture (la dernière heure n'est pas observable).

## Mesures, pour chaque relevé t d'une grille retenue
- `h` = heures entre `split_at` et la clôture ; `a` = âge ; tranches de `h` : [1, 3), [3, 6), [6, 12), [12, 24), [24, 48), ≥ 48 ;
  tranches de `a` : [0, 3), [3, 6), [6, 12), [12, 24), ≥ 24.
- **A (public)** : écart absolu moyen, en points, entre q(t) et `q_ref` sur toutes les rencontres et tous les signes ; écart maximal.
- **B (valeur)** : avec les cotes figées (archive la plus récente avant le dernier relevé), le modèle de production gelé (`FROZEN`,
  scénario S3, μ = 10, mêmes mises estimées) et q(t), la recherche de production donne 16 tickets ; `EV_t` = leur espérance S3 avec
  q(t), `EV_ref` = l'espérance S3 des **mêmes** tickets avec `q_ref`.
  - principale : `R = EV_t / EV_ref` du meilleur ticket ;
  - secondaire : rapport des sommes de (EV − 1) des tickets d'espérance S3 ≥ 1,7 au relevé t (seuil en production).
  Le calcul reprend le code de production ; avant l'analyse, il est contrôlé sur une séance enregistrée (même espérance attendue).

## Analyse
- Par tranche de `h`, puis de `a` : moyenne et médiane de A et de log R ; intervalle à 95 % par bootstrap des grilles (la grille est
  l'unité tirée, 10 000 tirages).
- Variable explicative : comparaison descriptive des deux régressions log R ~ log(1 + a) et log R ~ log(1 + h) (R², sans test).
- **Délai minimal H\*** : en partant de la tranche la plus proche de la clôture, la plus grande suite de tranches de `h` dont la borne haute
  de l'intervalle de la moyenne de R reste ≤ 1,10 (espérance gonflée de 10 % au plus) ; H\* = borne haute de la dernière tranche de
  cette suite. Même règle sur `a` (âge minimal A\*).
- **Enveloppe des espérances aberrantes** (seulement au second jalon) : 95e centile de R par tranche d'âge et de `h` ; une espérance
  dont le rapport attendu dépasse ce centile serait signalée.

## Jalons
- **Bilan d'étape** à 40 grilles retenues (environ 6 semaines au rythme 2025-2026, vers le 2026-12-05) : questions 1 et 2, H\* et A\*.
- **Enveloppe** à 100 grilles retenues (vers le 2027-02-06) : question 3.
- Aucune règle de production ne change sans plan séparé et go de Paul.

## Limites annoncées
Référence prise environ une heure avant la clôture et non la répartition finale FDJ (non relevée : pas de navigation automatisée chez un
opérateur) ; prono-match est un intermédiaire ; la trêve hivernale ralentit l'accumulation ; la dérive des cotes n'est pas mesurée
(cotes figées).

## Compter les grilles closes relevées (avant d'appliquer les critères d'inclusion)
```sql
SELECT g.game, COUNT(*) FROM lotofoot_grid g
WHERE g.closes_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
  AND (SELECT COUNT(DISTINCT s.split_at) FROM lotofoot_split s WHERE s.game = g.game AND s.number = g.number) >= 6
GROUP BY g.game;
```
