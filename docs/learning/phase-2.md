# Phase 2 — Historique et marché

Ce qui a été construit : l'import de 20 513 matchs et 284 119 lignes de cotes (football-data.co.uk, six
championnats, 2016/17 à aujourd'hui), une table d'alias des noms d'équipes, un contrôle qualité, trois méthodes de
retrait de marge, les scores d'évaluation (log loss, Brier, RPS), un bootstrap par blocs et un rapport HTML sur la
qualité du marché (`edge market report`).

## 1. Concepts Rust rencontrés

**Énumération et `match` pour choisir une méthode** — `crates/core/src/market/margin.rs:19` et `:30`.
`MarginMethod` est un `enum` à trois variantes ; `const ALL: [MarginMethod; 3]` permet de les parcourir toutes
(`for method in MarginMethod::ALL`). Pas de trait : trois façons de faire la même chose, c'est un choix fermé.

**Fermeture passée à une fonction (`impl Fn`)** — `crates/core/src/market/margin.rs:102`.
`bisect(f, lo, hi)` cherche la racine de n'importe quelle fonction décroissante. `power` et `shin` lui passent
chacune leur fonction sous forme de fermeture (`|k| …`). `impl Fn(f64) -> f64` se lit « n'importe quoi qui se
comporte comme une fonction de f64 vers f64 » : c'est le seul générique de la phase, et il évite de copier deux fois
l'algorithme.

**Motif sur tranche et `let … else`** — `crates/core/src/ingest/football_data/parse.rs:240`.
`let [Some(home), Some(draw), Some(away)] = values.as_slice() else { continue };` ne réussit que si la liste a
exactement trois éléments, tous présents ; sinon on passe au bookmaker suivant. Le `else` doit sortir de la
portée (`continue`, `return`) : le compilateur garantit que `home`, `draw` et `away` existent après la ligne.

**Fonction générique avec contrainte (`T: FromStr`)** — `crates/core/src/ingest/football_data/parse.rs:119`.
`Row::number::<u8>("FTHG")` et `Row::number::<f64>("PSH")` partagent le même code : `T: FromStr` dit « tout type
qu'on sait lire depuis un texte ». À l'appel, le type se précise avec `::<u8>` ou se déduit du champ à remplir.

**Structure qui emprunte (`Row<'a>`)** — `crates/core/src/ingest/football_data/parse.rs:103`.
`Row` ne possède ni l'en-tête ni la ligne : elle les emprunte pour la durée de l'analyse d'une ligne. Aucune copie
des 20 000 lignes, et le compilateur interdit de garder la `Row` plus longtemps que les données.

**Décoder du Latin-1 sans dépendance** — `crates/core/src/ingest/football_data/parse.rs:51` et `:54`.
En Latin-1, chaque octet est exactement le code Unicode du même numéro : `char::from(octet)` suffit. On essaie
d'abord l'UTF-8 (`from_utf8`), et on ne se rabat sur Latin-1 que si le fichier n'est pas valide.

**Adaptateurs d'itérateurs avancés** — `crates/core/src/ingest/football_data/mod.rs:113` (`flat_map` avec une
fermeture `move`), `crates/core/src/market/calibration.rs:117` (`windows(2)` : couples de bornes consécutives),
`crates/core/src/report/market_report.rs:357` (`min_by` avec `total_cmp`, car un `f64` n'est pas `Ord` à cause de NaN).

**L'API « entry » d'une table de hachage** — `crates/core/src/market/dataset.rs:70`.
`blocks.entry(clé).or_insert(valeur)` donne l'identifiant du bloc s'il existe, sinon l'ajoute : un seul passage,
sans le « regarder puis insérer » en deux temps.

**Désérialisation et sérialisation avec `serde`** — `crates/core/src/market/margin.rs:19` (`Deserialize` : la
configuration écrit `margin_method = "power"`), `crates/core/src/report/render.rs:42` (`Serialize` : les
structures du rapport deviennent le contexte du gabarit HTML). Les fonctions `number` et `percent`
(`render.rs:14`) sont de simples fonctions Rust enregistrées comme filtres du gabarit (`render.rs:36`).

**Générateur pseudo-aléatoire sans dépendance** — `crates/core/src/eval/bootstrap.rs:29` à `:36`.
`wrapping_add` et `wrapping_mul` calculent « modulo 2⁶⁴ » sans panique en cas de dépassement (en mode debug,
`+` et `*` plantent sur un dépassement ; ici le dépassement est voulu).

**Écriture atomique d'un fichier** — `crates/core/src/ingest/football_data/fetch.rs:109`.
On écrit dans `fichier.csv.part` puis on renomme : un téléchargement interrompu ne laisse jamais un fichier
tronqué que l'on prendrait pour complet.

## 2. Concepts statistiques

**Retrait de marge** (`crates/core/src/market/margin.rs`). Une cote `c` donne la probabilité implicite `π = 1/c`.
Pour un 1X2, `β = Σπ > 1` ; l'excédent `β − 1` est la marge. Trois façons de ramener la somme à 1 :
multiplicative `p = π/β` ; puissance `p = π^k` avec `Σπ^k = 1` ; Shin, qui suppose une fraction `z` de parieurs
informés. Les deux dernières retirent *plus* de marge aux outsiders que la première (biais favori / outsider).

**Log loss, Brier, RPS** (`crates/core/src/eval/metrics.rs:10`, `:16`, `:28`). Trois scores propres : plus bas =
meilleur. Log loss `−ln p_réalisé` (pénalise sévèrement l'excès de confiance) ; Brier `Σ(p_i − o_i)²` ; RPS, qui
tient compte de l'ordre domicile < nul < extérieur : rater un nul coûte moins cher que rater une victoire adverse.
Jamais le « taux de bons pronostics ».

**Calibration et intervalle de Wilson** (`crates/core/src/market/calibration.rs:18`, `:51`). Parmi les prévisions à
30 %, environ 30 % doivent se réaliser. Le diagramme de fiabilité compare, par classe de 5 points, probabilité
prévue et fréquence observée ; l'intervalle de Wilson donne la marge d'erreur de cette fréquence.

**Bootstrap par blocs** (`crates/core/src/eval/bootstrap.rs:50`). Pour dire si un écart est du bruit, on
rééchantillonne 2 000 fois, mais par blocs (une ligue et une semaine) : les matchs d'une même journée ne sont pas
indépendants, un rééchantillonnage match par match donnerait des intervalles trop étroits.

**Ce que les données disent (mesuré le 2026-10-18, `edge market report`).**
- Marge de clôture : Pinnacle 2,6 %, moyenne du marché 4,9 %, Bet365 5,7 %. La marge de Pinnacle est stable
  autour de 2 à 3 % depuis 2016.
- Les trois méthodes de retrait donnent des scores quasi identiques pour Pinnacle (log loss 0,958) ; sur des
  bookmakers à marge plus élevée, `power` et `shin` battent systématiquement la multiplicative. Retenue : `power`.
- Le marché est remarquablement calibré : les points du diagramme suivent la diagonale.
- La moyenne du marché à la clôture est indiscernable de Pinnacle à la clôture (écart de log loss −0,00009,
  intervalle [−0,00047 ; +0,00028]) : le repli après la disparition de Pinnacle est justifié.
- Le relevé d'avant-match est **significativement moins bon** que la clôture (+0,0023 de log loss) : le marché
  apprend entre le vendredi et le coup d'envoi. C'est la raison d'être de la mesure du CLV.
- Rendement d'une mise à la cote brute : −0,0 % sur les favoris (cote juste < 1,5), −4,1 % entre 3 et 5,
  **−14,8 % au-delà de 10** : les outsiders coûtent bien plus cher que la marge ne l'explique.

## 3. Trois questions pour vérifier que tu as compris

1. Pourquoi compare-t-on les trois méthodes de retrait de marge sur *exactement les mêmes* matchs, et pourquoi
   l'unité du bootstrap est-elle « une ligue et une semaine » plutôt qu'un match ?
2. Le marché est calibré (les points suivent la diagonale), et pourtant parier sur chaque outsider à cote juste
   supérieure à 10 rend −14,8 %. Pourquoi ces deux faits ne se contredisent-ils pas, et que doit faire un modèle
   pour gagner ?
3. Pourquoi l'import s'arrête-t-il sur un nom d'équipe inconnu au lieu de proposer le plus ressemblant ? Dans
   `let [Some(home), Some(draw), Some(away)] = values.as_slice() else { continue };`, que se passe-t-il si Betfair
   n'a publié que la cote domicile ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/market/margin.rs` — les trois formules, la bissection et les tests contre des valeurs de
   référence calculées à part : c'est le cœur mathématique de la phase, réutilisé par tous les modèles.
2. `crates/core/src/ingest/football_data/parse.rs` — la réalité des données : BOM, Latin-1, dates à deux
   chiffres, heure britannique, colonnes qui changent. Ses tests racontent chaque piège.
3. `crates/core/src/report/market_report.rs` — comment on juge un pronostiqueur : mêmes matchs, scores propres,
   intervalles par blocs. La phase 3 réutilisera cette mécanique pour juger les modèles contre le marché.
