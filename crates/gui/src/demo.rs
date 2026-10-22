//! Mode démonstration (`edge-gui --demo`) : des données **fictives**, des clubs imaginaires, rien lu ni écrit en base.
//!
//! Il sert à juger l'interface (et à la photographier) sans toucher au journal réel. Chaque écran affiche un repère « DÉMO ».

use chrono::{DateTime, Duration, Utc};
use edge_core::betting::clv::{Bucket, ClvReport, Group};
use edge_core::betting::ledger::UpcomingEvent;
use edge_core::betting::live::Opportunity;
use edge_core::betting::overview::{BankrollCurve, CurvePoint, ReliabilityBin};
use edge_core::betting::value::Selection;
use edge_core::bonus::OfferKind;
use edge_core::bonus::cover::cover;
use edge_core::bonus::pick::{Candidate, Target};
use edge_core::bonus::tracking::{self, Board, Plan};
use edge_core::config::Config;
use edge_core::domain::{Euros, Odds, Outcome, Probability};
use edge_core::eval::bootstrap::MeanCi;
use edge_core::format::{kickoff_label, long_date};
use edge_core::report::lotofoot_page::{BandView, GridView, LedgerView, LotofootPage, TicketView};
use edge_core::store::bonus::{
    Eligibility, EventRef, Leg, MatchResult, NewOffer, Role, Settled, StoredOffer, StoredOperation,
};
use edge_core::store::journal::{
    BetResult, NewBet, NewRecommendation, Settlement, SettlementSource, StoredBet,
    StoredRecommendation, Tier,
};

use crate::data::{Credits, Data, Feed, ReportFile, ScanSummary};

fn odds(value: f64) -> Odds {
    Odds::new(value).expect("cote de démonstration valide")
}

fn probability(value: f64) -> Probability {
    Probability::new(value).expect("probabilité de démonstration valide")
}

fn euros(value: f64) -> Euros {
    Euros::new(value).expect("montant de démonstration valide")
}

/// Un match imaginaire : (domicile, extérieur, clé de ligue).
const FIXTURES: [(&str, &str, &str); 6] = [
    ("FC Aurore", "Olympique Brume", "soccer_france_ligue_one"),
    ("Union Cobalt", "Real Estuaire", "soccer_spain_la_liga"),
    ("SC Rivage", "Dynamo Falaise", "soccer_epl"),
    (
        "Atletico Mistral",
        "Sporting Lagune",
        "soccer_spain_la_liga",
    ),
    (
        "Borussia Quartz",
        "Inter Zéphyr",
        "soccer_germany_bundesliga",
    ),
    ("Celtic Brumaire", "Hearts Aubépine", "soccer_spl"),
];

/// Un prix d'opérateur face à la cote juste de Pinnacle.
#[derive(Clone, Copy)]
struct Quote<'a> {
    bookmaker: &'a str,
    outcome: Outcome,
    price: f64,
    fair: f64,
}

/// Un pari placé : issue, cote obtenue, mise.
#[derive(Clone, Copy)]
struct Wager {
    outcome: Outcome,
    price: f64,
    stake: f64,
}

const fn quote(bookmaker: &str, outcome: Outcome, price: f64, fair: f64) -> Quote<'_> {
    Quote {
        bookmaker,
        outcome,
        price,
        fair,
    }
}

fn opportunity(index: usize, now: DateTime<Utc>, quote: Quote, stake: Option<f64>) -> Opportunity {
    let (home, away, league) = FIXTURES[index];
    let p = probability(1.0 / quote.fair);
    let edge = p.get() * quote.price - 1.0;
    Opportunity {
        event_id: format!("demo-{index}"),
        api_call_id: 1,
        sport_key: league.to_string(),
        home_team: home.to_string(),
        away_team: away.to_string(),
        kickoff: now + Duration::hours(5 + 9 * index as i64),
        bookmaker: quote.bookmaker.to_string(),
        selection: Selection {
            outcome: quote.outcome,
            odds: odds(quote.price),
            probability: p,
            edge,
        },
        minimal_odds: 1.03 / p.get(),
        priced_at: now - Duration::hours(2),
        stake: stake.map(euros),
    }
}

fn recommendation(
    id: i64,
    index: usize,
    now: DateTime<Utc>,
    tier: Tier,
    quote: Quote,
    stake: Option<f64>,
    days_ago: i64,
) -> StoredRecommendation {
    let o = opportunity(index, now, quote, stake);
    StoredRecommendation {
        id,
        created_at: now - Duration::days(days_ago),
        record: NewRecommendation {
            rule: "price_vs_pinnacle".into(),
            tier,
            event_id: o.event_id,
            sport_key: o.sport_key,
            home_team: o.home_team,
            away_team: o.away_team,
            kickoff: o.kickoff - Duration::days(days_ago),
            bookmaker: o.bookmaker,
            outcome: quote.outcome,
            odds: o.selection.odds,
            probability: o.selection.probability,
            edge: o.selection.edge,
            min_edge: if tier == Tier::Recommended { 0.03 } else { 0.0 },
            minimal_odds: o.minimal_odds,
            stake: o.stake,
            kelly_fraction: 1.0,
            bankroll: euros(1000.0),
            margin_method: "power".into(),
            api_call_id: 1,
            code_version: "démo".into(),
        },
    }
}

fn bet(
    id: i64,
    index: usize,
    now: DateTime<Utc>,
    recommendation_id: Option<i64>,
    wager: Wager,
    days_ago: i64,
    result: Option<(Settlement, f64)>,
) -> StoredBet {
    let (home, away, league) = FIXTURES[index % FIXTURES.len()];
    let kickoff = now - Duration::days(days_ago) + Duration::hours(3);
    StoredBet {
        id,
        created_at: now - Duration::days(days_ago) - Duration::hours(6),
        bet: NewBet {
            recommendation_id,
            event_id: format!("demo-bet-{id}"),
            sport_key: league.to_string(),
            home_team: home.to_string(),
            away_team: away.to_string(),
            kickoff,
            bookmaker: if id % 2 == 0 {
                "betclic_fr"
            } else {
                "winamax_fr"
            }
            .to_string(),
            outcome: wager.outcome,
            odds: odds(wager.price),
            stake: euros(wager.stake),
        },
        result: result.map(|(settlement, profit)| BetResult {
            settled_at: kickoff + Duration::hours(2),
            settlement,
            source: SettlementSource::FootballData,
            profit,
        }),
    }
}

fn interval(mean: f64, half_width: f64) -> MeanCi {
    MeanCi {
        mean,
        low: mean - half_width,
        high: mean + half_width,
    }
}

/// Les données fictives affichées en mode démo.
pub fn data(now: DateTime<Utc>) -> Data {
    use Outcome::{Away, Draw, Home};
    let notified = vec![
        opportunity(0, now, quote("betclic_fr", Home, 2.30, 2.06), Some(91.4)),
        opportunity(1, now, quote("winamax_fr", Draw, 3.55, 3.40), Some(34.2)),
        opportunity(2, now, quote("unibet_fr", Away, 4.10, 3.95), Some(21.7)),
    ];
    let below = vec![
        opportunity(3, now, quote("pmu_fr", Draw, 3.30, 3.26), None),
        opportunity(4, now, quote("betclic_fr", Home, 1.92, 1.90), None),
    ];
    let recommendations = vec![
        recommendation(
            9,
            0,
            now,
            Tier::Recommended,
            quote("betclic_fr", Home, 2.30, 2.06),
            Some(91.4),
            0,
        ),
        recommendation(
            8,
            1,
            now,
            Tier::Recommended,
            quote("winamax_fr", Draw, 3.55, 3.40),
            Some(34.2),
            0,
        ),
        recommendation(
            7,
            3,
            now,
            Tier::Shadow,
            quote("pmu_fr", Draw, 3.30, 3.26),
            None,
            1,
        ),
        recommendation(
            6,
            4,
            now,
            Tier::Recommended,
            quote("unibet_fr", Away, 3.80, 3.62),
            Some(40.0),
            2,
        ),
        recommendation(
            5,
            2,
            now,
            Tier::Shadow,
            quote("betclic_fr", Home, 1.92, 1.90),
            None,
            3,
        ),
    ];
    let wager = |outcome, price, stake| Wager {
        outcome,
        price,
        stake,
    };
    let bets = vec![
        bet(8, 0, now, Some(9), wager(Home, 2.30, 50.0), 0, None),
        bet(7, 1, now, None, wager(Draw, 3.40, 5.0), 1, None),
        bet(
            6,
            2,
            now,
            Some(6),
            wager(Away, 3.80, 40.0),
            2,
            Some((Settlement::Won, 112.0)),
        ),
        bet(
            5,
            3,
            now,
            Some(4),
            wager(Home, 2.05, 60.0),
            4,
            Some((Settlement::Lost, -60.0)),
        ),
        bet(
            4,
            4,
            now,
            Some(3),
            wager(Draw, 3.30, 35.0),
            6,
            Some((Settlement::Won, 80.5)),
        ),
        bet(
            3,
            5,
            now,
            None,
            wager(Away, 2.60, 20.0),
            8,
            Some((Settlement::Void, 0.0)),
        ),
        bet(
            2,
            0,
            now,
            Some(2),
            wager(Home, 1.95, 70.0),
            10,
            Some((Settlement::Lost, -70.0)),
        ),
        bet(
            1,
            1,
            now,
            Some(1),
            wager(Home, 2.20, 45.0),
            12,
            Some((Settlement::Won, 54.0)),
        ),
    ];

    let closes = [
        1000.0, 1000.0, 1054.0, 1054.0, 984.0, 984.0, 1064.5, 1064.5, 1064.5, 1052.0, 1052.0,
        1096.0, 1041.0, 1041.0, 1078.0, 1112.0, 1112.0, 1066.0, 1066.0, 1121.0, 1121.0, 1148.0,
        1103.0, 1103.0, 1099.0, 1142.0, 1142.0, 1186.0, 1186.0, 1213.0,
    ];
    let points: Vec<CurvePoint> = closes
        .iter()
        .enumerate()
        .map(|(i, bankroll)| CurvePoint {
            at: now - Duration::days(32 - i as i64),
            bankroll: *bankroll,
        })
        .collect();

    let groups = vec![
        Group {
            label: "Recommandations notifiées",
            n: 14,
            with_closing: 11,
            settled: 9,
            clv: Some(interval(0.0142, 0.021)),
            flat_roi: Some(interval(0.061, 0.19)),
            money: None,
        },
        Group {
            label: "Journal fantôme (sous le seuil)",
            n: 62,
            with_closing: 48,
            settled: 41,
            clv: Some(interval(0.0031, 0.009)),
            flat_roi: Some(interval(-0.012, 0.08)),
            money: None,
        },
        Group {
            label: "Mes paris suivant une recommandation",
            n: 6,
            with_closing: 5,
            settled: 5,
            clv: Some(interval(0.0118, 0.034)),
            flat_roi: Some(interval(0.14, 0.4)),
            money: Some((255.0, 116.5)),
        },
        Group {
            label: "Mes paris hors modèle",
            n: 2,
            with_closing: 1,
            settled: 1,
            clv: Some(interval(-0.012, 0.0)),
            flat_roi: None,
            money: Some((20.0, 0.0)),
        },
    ];
    let buckets = vec![
        Bucket {
            label: "0 % à 1 %",
            n: 31,
            with_closing: 24,
            mean_edge: 0.0048,
            mean_clv: Some(-0.0021),
        },
        Bucket {
            label: "1 % à 2 %",
            n: 18,
            with_closing: 14,
            mean_edge: 0.0151,
            mean_clv: Some(0.0052),
        },
        Bucket {
            label: "2 % à 3 %",
            n: 13,
            with_closing: 10,
            mean_edge: 0.0247,
            mean_clv: Some(0.0088),
        },
        Bucket {
            label: "3 % à 5 %",
            n: 11,
            with_closing: 9,
            mean_edge: 0.0381,
            mean_clv: Some(0.0143),
        },
        Bucket {
            label: "≥ 5 %",
            n: 3,
            with_closing: 2,
            mean_edge: 0.0612,
            mean_clv: Some(0.0231),
        },
    ];
    let reliability = vec![
        ReliabilityBin {
            low: 0.2,
            high: 0.4,
            n: 14,
            mean_probability: 0.29,
            frequency: 0.26,
        },
        ReliabilityBin {
            low: 0.4,
            high: 0.6,
            n: 18,
            mean_probability: 0.47,
            frequency: 0.50,
        },
        ReliabilityBin {
            low: 0.6,
            high: 0.8,
            n: 9,
            mean_probability: 0.68,
            frequency: 0.63,
        },
    ];

    Data {
        now,
        config: Config::default(),
        scan: ScanSummary {
            events: 62,
            without_reference: 10,
            implausible_quotes: 0,
        },
        notified,
        below,
        recommendations,
        bets,
        upcoming: vec![UpcomingEvent {
            event_id: "demo-0".into(),
            sport_key: "soccer_france_ligue_one".into(),
            home_team: "FC Aurore".into(),
            away_team: "Olympique Brume".into(),
            kickoff: now + Duration::hours(5),
        }],
        curve: BankrollCurve {
            initial: 1000.0,
            settled_profit: 213.0,
            open_stakes: 55.0,
            available: 1158.0,
            max_drawdown: 0.062,
            points,
        },
        clv: ClvReport {
            groups,
            buckets,
            by_bookmaker: Vec::new(),
            by_outcome: Vec::new(),
        },
        reliability,
        reports: vec![
            ReportFile {
                name: "backtest.html".into(),
                path: "backtest.html".into(),
                modified: None,
            },
            ReportFile {
                name: "tennis.html".into(),
                path: "tennis.html".into(),
                modified: None,
            },
        ],
        feeds: vec![
            Feed {
                label: "Football",
                last: Some(now - Duration::hours(11)),
                archived: Some(20_513),
            },
            Feed {
                label: "Tennis",
                last: Some(now - Duration::hours(11)),
                archived: Some(68_251),
            },
            Feed {
                label: "NBA",
                last: Some(now - Duration::hours(2)),
                archived: Some(34),
            },
        ],
        credits: Some(Credits {
            used: 187,
            cap: 450,
        }),
        bonus: bonus_board(now),
        lotofoot: lotofoot_page(now),
    }
}

/// Clubs imaginaires des grilles Loto Foot de démonstration.
const LOTOFOOT_CLUBS: [(&str, &str); 15] = [
    ("FC Aurore", "Olympique Brume"),
    ("Union Cobalt", "Real Estuaire"),
    ("SC Rivage", "Dynamo Falaise"),
    ("Atletico Mistral", "Sporting Lagune"),
    ("Borussia Quartz", "Inter Zéphyr"),
    ("Celtic Brumaire", "Hearts Aubépine"),
    ("AS Galet", "Racing Embrun"),
    ("Stade Ondée", "US Marée"),
    ("FC Lichen", "AC Bruyère"),
    ("Rapid Ardoise", "Vitesse Argile"),
    ("Athletic Grêle", "Unión Sirocco"),
    ("Spartak Givre", "Lokomotiv Aube"),
    ("Real Cormoran", "CD Albatros"),
    ("Hertha Basalte", "Fortuna Silex"),
    ("Royal Écume", "Standard Varech"),
];

/// Loto Foot fictif : une séance de quatre tickets sur un LF7 et un LF8, le premier déjà déclaré joué.
fn lotofoot_page(now: DateTime<Utc>) -> LotofootPage {
    let saturday = now - Duration::hours(3);
    let grid = |name: &str, closes: DateTime<Utc>, clubs: &[(&str, &str)]| GridView {
        grid: name.to_string(),
        closes: kickoff_label(closes),
        matches: clubs
            .iter()
            .enumerate()
            .map(|(i, (h, a))| format!("{}. {h} – {a}", i + 1))
            .collect(),
    };
    let lf7_closes = now + Duration::hours(26);
    let lf8_closes = now + Duration::hours(28);
    let ticket =
        |id: i64, lf8: bool, signs: &str, ev: &str, ev_low: &str, paid: &str, played: bool| {
            TicketView {
                id,
                grid: if lf8 { "LF8 n°147" } else { "LF7 n°133" }.to_string(),
                closes: kickoff_label(if lf8 { lf8_closes } else { lf7_closes }),
                ticket: signs.to_string(),
                ev: ev.to_string(),
                ev_low: ev_low.to_string(),
                paid: paid.to_string(),
                status: "en attente".to_string(),
                played,
            }
        };
    let ledger =
        |label, tickets, settled, staked: &str, returned: &str, roi: &str, paid: &str| LedgerView {
            label,
            tickets,
            settled,
            staked: staked.to_string(),
            returned: returned.to_string(),
            roi: roi.to_string(),
            paid: paid.to_string(),
        };
    LotofootPage {
        nav: false,
        active: "lotofoot",
        generated_at: long_date(now),
        session: long_date(saturday),
        grids: vec![
            grid("LF7 n°133", lf7_closes, &LOTOFOOT_CLUBS[..7]),
            grid("LF8 n°147", lf8_closes, &LOTOFOOT_CLUBS[7..]),
        ],
        tickets: vec![
            ticket(41, false, "1N21N21", "1,84", "1,31", "2,4 %", true),
            ticket(42, false, "2N21N21", "1,62", "1,22", "1,9 %", false),
            ticket(43, true, "21N12222", "2,71", "1,58", "0,6 %", false),
            ticket(44, true, "21N1N222", "2,35", "1,40", "0,5 %", false),
        ],
        stake: "4 €".to_string(),
        skipped: vec!["LF7 n°134 : répartition pas encore publiée".to_string()],
        unknown_names: Vec::new(),
        ledgers: vec![
            ledger(
                "Tickets recommandés (suivi du modèle, joués ou non)",
                18,
                14,
                "14 €",
                "21,90 €",
                "+56,4 %",
                "1 / 0,3",
            ),
            ledger("Tickets joués du modèle", 15, 11, "11 €", "21,90 €", "+99,1 %", "1"),
            ledger("Tickets joués hors modèle", 0, 0, "0 €", "0,00 €", "—", "0"),
        ],
        verdict: "Pas de verdict avant 26 semaines (4 écoulée(s) depuis la première séance). Le critère est figé : \
                  calibration des concurrents, et gain réalisé au-dessus du 5e centile de sa loi simulée."
            .to_string(),
        verdict_class: "warn",
        bands: vec![
            BandView {
                range: "3 – 10".to_string(),
                observations: 6,
                ratio: "1,12".to_string(),
                failing: false,
            },
            BandView {
                range: "≥ 10".to_string(),
                observations: 4,
                ratio: "0,94".to_string(),
                failing: false,
            },
        ],
        gain: "21,90 € réalisés pour 17,64 € attendus ; centile 0,712 de la loi simulée (14 ticket(s) réglé(s))"
            .to_string(),
        frequency: "1 payé(s) pour 0,30 attendu(s) ; probabilité d'en avoir aussi peu : 1,000".to_string(),
    }
}

/// Bonus fictifs : Winamax inéligible, PMU consommé et réglé, Betclic et bet365 à jouer.
fn bonus_board(now: DateTime<Utc>) -> Board {
    let offer = |id: i64, bookmaker: &str, eligibility: Eligibility| StoredOffer {
        id,
        created_at: now - Duration::days(3),
        offer: NewOffer {
            bookmaker: bookmaker.into(),
            eligibility,
            checked_on: (now - Duration::days(3)).date_naive(),
            source: "règlement (démo)".into(),
            note: None,
        },
    };
    let eligible = |kind, amount| Eligibility::Eligible {
        kind,
        amount: euros(amount),
        min_odds: None,
    };
    let offers = vec![
        offer(1, "winamax_fr", Eligibility::Ineligible),
        offer(2, "pmu_fr", eligible(OfferKind::CashRefund, 100.0)),
        offer(3, "betclic_fr", eligible(OfferKind::FreebetRefund, 100.0)),
        offer(4, "bet365_fr", eligible(OfferKind::FreebetRefund, 100.0)),
    ];
    let leg = |role, bookmaker: &str, outcome, price: f64, stake: f64| Leg {
        role,
        bookmaker: bookmaker.into(),
        outcome,
        odds: odds(price),
        stake: euros(stake),
    };
    let event = |index: usize, days: i64| EventRef {
        event_id: format!("demo-{index}"),
        sport_key: FIXTURES[index].2.into(),
        home_team: FIXTURES[index].0.into(),
        away_team: FIXTURES[index].1.into(),
        kickoff: now + Duration::days(days),
    };
    let pmu = tracking::new_operation(
        2,
        OfferKind::CashRefund,
        None,
        event(1, -2),
        vec![
            leg(Role::Bonus, "pmu_fr", Outcome::Away, 3.35, 100.0),
            leg(Role::Cover, "winamax_fr", Outcome::Home, 1.98, 118.7),
            leg(Role::Cover, "winamax_fr", Outcome::Draw, 3.75, 62.7),
        ],
    );
    let operations = vec![StoredOperation {
        id: 1,
        created_at: now - Duration::days(3),
        operation: pmu,
        settled: Some(Settled {
            settled_at: now - Duration::days(1),
            result: MatchResult::Played(Outcome::Home),
            source: SettlementSource::FootballData,
        }),
    }];
    let operators = tracking::statuses(&offers, &operations);
    let cover_accounts = tracking::cover_accounts(&operators);

    let target = |bookmaker: &str| Target {
        bookmaker: bookmaker.into(),
        kind: OfferKind::FreebetRefund,
        amount: euros(100.0),
        min_odds: None,
        freebet_value: 0.53,
    };
    let candidate = |index: usize, bonus: (Outcome, f64), covers: [(&str, Outcome, f64); 2]| {
        let others = [odds(covers[0].2), odds(covers[1].2)];
        let result = cover(
            OfferKind::FreebetRefund,
            euros(100.0),
            odds(bonus.1),
            0.53,
            &others,
        )
        .expect("cotes de démonstration valides");
        let mut legs = vec![leg(Role::Bonus, "betclic_fr", bonus.0, bonus.1, 100.0)];
        for ((bookmaker, outcome, price), stake) in covers.into_iter().zip(&result.stakes) {
            legs.push(leg(Role::Cover, bookmaker, outcome, price, stake.get()));
        }
        Candidate {
            event: event(index, 3),
            api_call_id: 1,
            priced_at: now - Duration::hours(6),
            legs,
            cover: result,
        }
    };
    let plans = vec![
        Plan {
            target: target("betclic_fr"),
            name: "Betclic".into(),
            in_archive: true,
            offer_id: 3,
            candidates: vec![
                candidate(
                    2,
                    (Outcome::Home, 2.63),
                    [
                        ("winamax_fr", Outcome::Draw, 3.40),
                        ("pmu_fr", Outcome::Away, 2.45),
                    ],
                ),
                candidate(
                    4,
                    (Outcome::Away, 3.00),
                    [
                        ("winamax_fr", Outcome::Home, 2.05),
                        ("pmu_fr", Outcome::Draw, 3.80),
                    ],
                ),
            ],
        },
        Plan {
            target: target("bet365_fr"),
            name: "bet365".into(),
            in_archive: false,
            offer_id: 4,
            candidates: Vec::new(),
        },
    ];
    let realized = operations.iter().filter_map(tracking::realized_gain).sum();
    Board {
        operators,
        cover_accounts,
        plans,
        operations,
        realized,
        open_guaranteed: 0.0,
    }
}
