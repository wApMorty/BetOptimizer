# Phase 8 — NBA

Ce qui a été construit : l'archive des cotes de la NBA (vainqueur, handicaps, totaux, chaque soir) et des résultats
(`edge nba snapshot`), un Elo à marge de victoire avec avantage du terrain, dos à dos et retour vers la moyenne entre saisons, un
modèle de total, des probabilités de handicap et de plus/moins par loi normale, un réglage gelé sur une validation aux dates
fixées d'avance, et `edge nba report` (le modèle contre Pinnacle). Aucun historique gratuit et licite n'existe (`docs/sources.md`
§23) : **la première comparaison utile arrivera en février 2027**. D'ici là, le rapport est exploratoire et le dit.

## 1. Concepts Rust rencontrés

**Chaînes de `if let` et `&&`.** `crates/core/src/report/nba_report.rs:158` (`lines`). `if let (Some(h), Some(a)) = … && let
Some(p) = fair_two_way(…)` lit un marché de Pinnacle : la condition ne passe que si toutes les pièces existent (deux handicaps
qui se répondent, une marge plausible). Un marché incomplet est *absent*, jamais deviné.

**Tri sur un couple.** `models/nba.rs:230` (`walk_forward`) : `sort_by(|a, b| (a.commence_time, &a.event_id).cmp(&(b.commence_time,
&b.event_id)))` donne un ordre total et stable (heure, puis identifiant) ; deux exécutions donnent exactement les mêmes prévisions.

**Table de comptage avec des clés empruntées.** `walk_forward` : `HashMap<&str, u32>` compte les matchs vus par équipe sans copier
les noms ; les clés empruntent les `StoredGame` qui vivent plus longtemps que la table.

**Une loi normale sans `unwrap`.** `models/nba.rs:269` et suivantes : `statrs::Normal::new(0.0, 1.0)` renvoie un `Result` ; un
`match` traite le cas impossible sans paniquer (CLAUDE.md : pas d'`unwrap` hors des tests).

**Module de test partagé.** `models/nba_tuning.rs:170` : `#[cfg(test)] pub(crate) mod simulation` fabrique une saison simulée
(générateur congruentiel, forces connues) utilisée par les tests du réglage **et** du rapport, sans exister dans le binaire.

**Une constante qui vérifie une date à la compilation.** `nba_tuning.rs:19` et `:25` : `match NaiveDate::from_ymd_opt(…)` dans un
`const`. Les fenêtres de validation et de test sont fixées dans le code, avant la moindre donnée.

**Accumulateur de crédits.** `ingest/nba.rs:117` (`fetch_scores`) : on n'appelle l'API que si le dernier appel date de plus de 40 h et
si la garde de budget l'accepte ; sinon on renvoie la raison dans le bilan, pas une erreur.

## 2. Concepts statistiques

**Elo à marge de victoire.** L'écart d'Elo (vu du domicile, avantage du terrain et repos compris) donne `P = 1/(1+10^(−écart/400))`. Après
le match, `Δ = K · mult · (S − P)` avec `mult = (marge+3)^0,8 / (7,5 + 0,006 · écart_du_vainqueur)` (`models/nba.rs:138`). Le numérateur
donne des rendements décroissants à la marge ; le dénominateur réduit le gain quand le favori gagne largement (ses points sont
« attendus » : sans cette correction, un favori qui gagne souvent gonflerait son classement).

**Avantage du terrain, repos, retour vers la moyenne.** Le terrain vaut 100 points d'Elo au départ, le dos à dos en retire une pénalité,
et chaque nouvelle saison ramène une part du classement vers 1500. Aucune de ces valeurs n'est « vraie » : elles viennent de
FiveThirtyEight (page disparue, recoupée par un résumé) et sont **réglées** sur la validation.

**De l'Elo à la marge.** La marge attendue est l'écart d'Elo × une pente `p` estimée par moindres carrés sans ordonnée
(`margin_per_elo`, `nba.rs:289`) ; l'écart-type `σ` est celui des résidus. Marge et total sont supposés gaussiens :
`P(handicap couvert) = Φ((marge + handicap)/σ)` (`nba.rs:269`).

**Total par tendances d'équipe.** `total prévu = moyenne + t_dom + t_ext`, chaque tendance bougeant de `taux × erreur` après un match
(`nba.rs:157`) : un modèle additif appris en ligne, plus simple qu'un modèle d'attaque et de défense, suffisant pour un premier total.

**Un seul marché français comparable.** Les opérateurs français ne cotent que le vainqueur (marge 7 à 10 % chez Betclic, Winamax,
Unibet, PMU) ; les handicaps et totaux n'existent que chez Pinnacle (3 %) comme référence. Les handicaps et totaux servent à
*juger le modèle*, pas à parier en France.

**Fenêtres fixées avant les données.** Chauffe jusqu'au 15/11/2026, validation jusqu'au 01/02/2027, test ensuite (`nba_tuning.rs:19`).
`edge nba tune` est refusé avant le 01/02/2027 : régler sur une validation incomplète, ou après avoir regardé le test, rendrait le
test inutile. Le rapport « exploratoire » (configuration par défaut, toute l'archive) décrit sans jamais servir à régler.

**Lignes de Pinnacle et égalités.** La ligne d'un handicap est la médiane de la marge selon Pinnacle ; une ligne entière peut tomber
juste (remboursement) : ces matchs sont exclus des log loss de handicap et de plus/moins.

## 3. Trois questions pour vérifier que tu as compris

1. Un favori de +300 d'Elo gagne de 25 points, puis un outsider gagne de 25 points contre ce même favori. Lequel des deux résultats
   déplace le plus les classements, et quelle partie de la formule le décide ?
2. Pourquoi la validation et le test sont-ils des dates écrites dans le code, et pourquoi `edge nba tune` refuse-t-il de tourner en
   décembre alors que les données existent déjà ?
3. La marge attendue du modèle est +3 pour l'équipe à domicile, σ = 12, le handicap de Pinnacle est −5,5. Quelle probabilité le modèle
   donne-t-il à « l'équipe à domicile couvre » ? Pinnacle, lui, donne environ 50 % : que cela te dit-il du modèle ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/models/nba.rs` — le modèle entier : Elo à marge, repos, total, loi normale, et ses tests (multiplicateur calculé à
   la main, non-fuite du futur, retour vers la moyenne).
2. `crates/core/src/models/nba_tuning.rs` — le protocole : fenêtres fixées, 57 configurations, estimation de la pente et des écarts-
   types, test sur une saison simulée dont les paramètres sont connus.
3. `crates/core/src/ingest/nba.rs` — comment l'archive se remplit pour 5 crédits par jour et au plus, et pourquoi un appel manqué ne
   perd aucun match (recouvrement d'un jour).
