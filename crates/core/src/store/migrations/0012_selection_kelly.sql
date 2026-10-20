-- Fraction de Kelly sous laquelle la croissance attendue a choisi le seuil gelé (critère `expected_growth`). Décision du
-- 2026-10-20 : demi-Kelly, car en Kelly complet la croissance de Bet365 est négative à tous les seuils. NULL pour `mean_clv`.

ALTER TABLE backtest_selection ADD COLUMN selection_kelly REAL;
