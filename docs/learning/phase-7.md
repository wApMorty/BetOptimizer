# Phase 7 — Tennis

Ce qui a été construit : la lecture des tableurs de tennis-data.co.uk (`calamine`), leur import et un contrôle qualité ; un Elo
général + par surface avec un `K` qui décroît avec l'expérience ; un réglage sur la validation 2022-2023, gelé avant le test ; un
rapport « l'Elo contre le marché » ; et l'archive des cotes de tennis en direct dans le relevé quotidien. Aucun backtest de pari :
les cotes historiques sont quasiment des clôtures (`docs/sources.md` §21).

## 1. Concepts Rust rencontrés

**Lire un format externe par nom de colonne.** `crates/core/src/ingest/tennis/parse.rs:43` et `:55`. Les colonnes d'un tableur
changent d'un fichier à l'autre : on construit une table `nom → indice` depuis l'en-tête (`HashMap<String, usize>`), puis chaque
ligne est lue par nom. Une colonne obligatoire absente est une erreur claire, pas un indice faux.

**Un `enum` de la bibliothèque, traité par `match`.** `parse.rs` (`date`, `number`, `text`). `calamine::Data` a une variante par
type de cellule (`Float`, `String`, `DateTime`, `Empty`…). Les fonctions d'aide traitent chaque cas et renvoient `None` pour les
autres : une cellule inattendue donne une ligne rejetée avec sa raison, jamais un plantage.

**Constante calculée à la compilation.** `crates/core/src/models/elo_tuning.rs:14`. `const VALIDATION_START: NaiveDate = match
NaiveDate::from_ymd_opt(2022, 1, 1) { Some(d) => d, None => panic!(…) }` : une date invalide empêcherait de compiler. C'est un
`panic!` qui ne peut jamais se produire à l'exécution, donc compatible avec la règle « pas de `unwrap()` ».

**Une énumération comme clé de `HashMap`.** `models/elo.rs:203`. `HashMap<Tour, Elo>` : `Tour` dérive `Hash` et `Eq`, ce qui suffit.
Un Elo par circuit, car les joueuses et les joueurs ne se rencontrent jamais.

**Parcourir des groupes consécutifs.** `elo.rs:203` (`walk_forward`) : `take_while(|m| m.date == day).count()` repère la fin de la
journée en cours dans un tableau déjà trié ; on prévoit toute la journée, puis on la met à jour.

**`Iterator::reduce`.** `elo_tuning.rs:80` : garde la meilleure ligne de la grille sans valeur initiale artificielle (et renvoie
`None` pour une grille vide).

**Fermetures passées sous forme d'objet.** `report/tennis_report.rs:249` : `&dyn Fn(&EvalRow) -> String` permet de découper les
résultats par surface, circuit ou format avec une seule fonction.

**Syntaxe de mise à jour de structure.** Tests de `elo.rs` : `EloConfig { surface_weight: 0.0, ..config() }` copie tous les autres
champs. Utile pour faire varier un seul paramètre dans un test.

## 2. Concepts statistiques

**Elo.** Chaque joueur a un nombre ; la probabilité que `a` batte `b` est logistique en l'écart : `1/(1 + 10^(−(r_a − r_b)/400))`
(`elo.rs:84`). Après un match, le vainqueur monte de `K·(1 − P(il gagnait))` : une surprise déplace beaucoup, un résultat attendu peu.

**`K` décroissant.** `K(n) = k_base/(n+5)^k_shape` (`elo.rs:63`) : un joueur nouveau bouge vite, un joueur installé lentement. C'est un
choix du projet réglé sur la validation, pas une formule publiée (la source citée ne donne pas de valeur).

**Classement de surface et mélange.** Chaque joueur a un Elo général et un Elo par surface ; la prévision utilise
`(1 − w)·général + w·surface`. Tennis Abstract trouve `w = 0,5` ; nous le réglons. Un Elo de surface naît égal à l'Elo général du joueur
(`elo.rs:111`), sinon un bon joueur serait sous-estimé sur une surface nouvelle.

**Aucune fuite, jour par jour.** Les prévisions d'une date sont toutes faites avant d'appliquer les résultats de cette date
(`elo.rs:203`). Un test réécrit l'avenir et vérifie que les prévisions passées ne bougent pas, **et** que les futures changent.

**Log loss contre le marché, orientation du vainqueur.** Tennis-data donne la cote du vainqueur et celle du perdant. Le log loss se
calcule sur la probabilité attribuée au vainqueur réel : c'est correct. Mais un diagramme de fiabilité sur « le vainqueur gagne » serait
vide de sens (toujours vrai) : on oriente sur le **favori** de chaque prévision (`tennis_report.rs:240`).

**Test d'information en logit.** On mélange marché et Elo par `σ((1 − w)·logit(m) + w·logit(e))` ; si le meilleur `w` (choisi sur la
validation) est 0, l'Elo ne sait rien que le marché ignore (`tennis_report.rs:235`).

**Homonymes.** Un joueur ne joue pas deux matchs le même jour : un nom qui apparaît deux fois à la même date désigne au moins deux
personnes (`ingest/tennis/quality.rs:158`). Le détecteur est exact pour ce cas, aveugle aux homonymes qui ne jouent jamais le même jour.

**Pourquoi pas de backtest de pari.** Une cote « la plus récente avant le match » est une clôture. Décider dessus, c'est connaître le
prix final : l'invariant 4 l'interdit, et le CLV n'a plus de référence distincte. L'archive en direct (`snapshot.rs:170`) sépare enfin
le prix d'avant-match de la clôture : elle seule pourra juger un avantage.

## 3. Trois questions pour vérifier que tu as compris

1. Deux matchs ont lieu le même jour : Alpha bat Bravo le matin, Alpha rejoue le soir (cas fictif). Pourquoi la prévision du second match
   ne doit-elle pas voir la victoire du matin, et quelle fuite créerait-on si elle la voyait ?
2. Le log loss de l'Elo est 0,64 et celui du marché 0,61, intervalle de l'écart [0,015 ; 0,045]. Que conclus-tu ? Et si l'intervalle
   était [−0,01 ; 0,05] ?
3. Tennis-data donne des cotes très proches de la clôture. Pourquoi cela empêche-t-il de mesurer un CLV historique en tennis, alors que
   c'était possible en football ? Qu'est-ce qui change avec l'archive en direct ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/models/elo.rs` — le modèle entier, ses formules et ses tests : symétrie, K décroissant, naissance d'un Elo de surface,
   non-fuite du futur.
2. `crates/core/src/report/tennis_report.rs` — comment on juge un modèle de tennis contre le marché : mêmes matchs, blocs de semaines,
   orientation du favori, test d'information.
3. `crates/core/src/ingest/tennis/parse.rs` — la frontière avec un format que nous ne contrôlons pas : lecture par nom, rejet motivé,
   cotes invalides comptées, jamais corrigées.
