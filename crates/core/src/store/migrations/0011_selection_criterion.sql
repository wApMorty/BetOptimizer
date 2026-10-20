-- Critère de gel des seuils. La première session (phase 4) a gelé le seuil au CLV moyen le plus élevé ; ce critère favorise
-- mécaniquement les seuils hauts (peu de paris, mais chacun très « mou »). La deuxième session gèle le seuil qui maximise la
-- croissance logarithmique totale attendue sous la clôture de Pinnacle : le total gagné, pas la moyenne par pari.
-- Les lignes déjà écrites gardent leur critère réel, `mean_clv`, par la valeur par défaut.

ALTER TABLE backtest_selection ADD COLUMN criterion TEXT NOT NULL DEFAULT 'mean_clv'
    CHECK (criterion IN ('mean_clv', 'expected_growth'));
-- Croissance logarithmique totale attendue sur la période de réglage, au seuil gelé (Kelly complet) ; NULL pour `mean_clv`.
ALTER TABLE backtest_selection ADD COLUMN selection_growth REAL;
