//! Reference tests for the indicators. Expected values were worked out independently
//! of the crate (plain textbook formulas) and are pasted in; tolerance is 1e-9.

use jarvis_market::Bar;
use jarvis_market::indicators::{self, Pivots};

const EPS: f64 = 1e-9;

// Twenty closes. The series below are SMA(5), EMA(5), RSI(14), MACD(3, 6, 3) and
// Bollinger(5, 2) over them.
const SMA5: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    Some(44.104),
    Some(44.202),
    Some(44.40399999999999),
    Some(44.657999999999994),
    Some(45.104),
    Some(45.45400000000001),
    Some(45.666),
    Some(45.852000000000004),
    Some(45.89),
    Some(45.978),
    Some(46.018),
    Some(46.040000000000006),
    Some(46.040000000000006),
    Some(46.2),
    Some(46.188),
    Some(46.06),
];
const EMA5: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    Some(44.104),
    Some(44.346),
    Some(44.59733333333333),
    Some(44.87155555555555),
    Some(45.19437037037037),
    Some(45.48958024691358),
    Some(45.62305349794239),
    Some(45.758702331961594),
    Some(45.70913488797439),
    Some(45.8994232586496),
    Some(46.02628217243306),
    Some(46.01752144828871),
    Some(46.02168096552581),
    Some(46.151120643683875),
    Some(46.17408042912258),
    Some(45.99605361941506),
];
const RSI14: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(70.46413502109705),
    Some(66.24961855355505),
    Some(66.48094183471265),
    Some(69.34685316290866),
    Some(66.29471265892624),
    Some(57.91502067008556),
];
const MACD_LINE: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    None,
    Some(0.24791666666666856),
    Some(0.3114583333333343),
    Some(0.3582291666666677),
    Some(0.41375744047618923),
    Some(0.4259093324829948),
    Some(0.32869081784499343),
    Some(0.2770140886392838),
    Some(0.12898467268877312),
    Some(0.2012620708937618),
    Some(0.19832370298215096),
    Some(0.10894232830197836),
    Some(0.0678857904444925),
    Some(0.12495334257474866),
    Some(0.08676984796773013),
    Some(-0.06354852124444932),
];
const MACD_SIGNAL: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(0.30586805555555685),
    Some(0.35981274801587304),
    Some(0.3928610402494339),
    Some(0.3607759290472137),
    Some(0.31889500884324873),
    Some(0.22393984076601092),
    Some(0.21260095582988636),
    Some(0.20546232940601866),
    Some(0.1572023288539985),
    Some(0.1125440596492455),
    Some(0.11874870111199709),
    Some(0.10275927453986361),
    Some(0.019605376647707146),
];
const MACD_HIST: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(0.05236111111111086),
    Some(0.05394469246031619),
    Some(0.03304829223356087),
    Some(-0.03208511120222024),
    Some(-0.041880920203964944),
    Some(-0.0949551680772378),
    Some(-0.01133888493612456),
    Some(-0.0071386264238677),
    Some(-0.04826000055202015),
    Some(-0.04465826920475301),
    Some(0.00620464146275157),
    Some(-0.01598942657213348),
    Some(-0.08315389789215646),
];
const BOLL_MID: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    Some(44.104),
    Some(44.202),
    Some(44.40399999999999),
    Some(44.657999999999994),
    Some(45.104),
    Some(45.45400000000001),
    Some(45.666),
    Some(45.852000000000004),
    Some(45.89),
    Some(45.978),
    Some(46.018),
    Some(46.040000000000006),
    Some(46.040000000000006),
    Some(46.2),
    Some(46.188),
    Some(46.06),
];
const BOLL_UPPER: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    Some(44.63550352773994),
    Some(44.99015226955202),
    Some(45.44949318505669),
    Some(45.926536164246016),
    Some(46.129951265899116),
    Some(46.373443309834826),
    Some(46.377460469738125),
    Some(46.31837324108486),
    Some(46.22057525618231),
    Some(46.422953930199526),
    Some(46.52418573666195),
    Some(46.53136544445047),
    Some(46.53136544445047),
    Some(46.51723808094238),
    Some(46.496648667581766),
    Some(46.573030213535226),
];
const BOLL_LOWER: &[Option<f64>] = &[
    None,
    None,
    None,
    None,
    Some(43.572496472260056),
    Some(43.413847730447976),
    Some(43.35850681494329),
    Some(43.38946383575397),
    Some(44.07804873410088),
    Some(44.53455669016519),
    Some(44.95453953026187),
    Some(45.385626758915144),
    Some(45.559424743817694),
    Some(45.53304606980048),
    Some(45.51181426333805),
    Some(45.54863455554954),
    Some(45.54863455554954),
    Some(45.88276191905763),
    Some(45.87935133241824),
    Some(45.54696978646478),
];
const CLOSES: &[f64] = &[
    44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.1, 45.42, 45.84, 46.08, 45.89, 46.03, 45.61,
    46.28, 46.28, 46.0, 46.03, 46.41, 46.22, 45.64,
];

fn assert_series(actual: &[Option<f64>], expected: &[Option<f64>]) {
    assert_eq!(actual.len(), expected.len(), "length");
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        match (a, e) {
            (None, None) => {}
            (Some(a), Some(e)) => assert!((a - e).abs() < EPS, "index {i}: got {a}, want {e}"),
            _ => panic!("index {i}: got {a:?}, want {e:?}"),
        }
    }
}

fn bar(open: f64, high: f64, low: f64, close: f64, tick_volume: u64) -> Bar {
    Bar {
        time: 0,
        open,
        high,
        low,
        close,
        tick_volume,
        spread: 0,
    }
}

fn bars_from(rows: &[(f64, f64, f64, f64)]) -> Vec<Bar> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| Bar {
            time: i as i64 * 3600,
            open: r.0,
            high: r.1,
            low: r.2,
            close: r.3,
            tick_volume: 1,
            spread: 0,
        })
        .collect()
}

#[test]
fn sma_matches_reference_and_warms_up() {
    assert_series(&indicators::sma(CLOSES, 5), SMA5);
}

#[test]
fn ema_is_seeded_with_the_sma() {
    let e = indicators::ema(CLOSES, 5);
    assert_series(&e, EMA5);
    assert_eq!(e[3], None);
    assert!(
        (e[4].unwrap() - 44.104).abs() < EPS,
        "seed is the SMA of the first 5"
    );
}

#[test]
fn rsi_uses_wilder_smoothing() {
    let r = indicators::rsi(CLOSES, 14);
    assert_series(&r, RSI14);
    assert!(
        r[..14].iter().all(Option::is_none),
        "first value at index `period`"
    );
}

#[test]
fn rsi_with_no_losses_is_100_and_flat_is_50() {
    let rising: Vec<f64> = (0..30).map(|i| 100.0 + f64::from(i)).collect();
    let r = indicators::rsi(&rising, 14);
    assert_eq!(r[14], Some(100.0));
    assert_eq!(*r.last().unwrap(), Some(100.0));

    let flat = vec![50.0; 30];
    let r = indicators::rsi(&flat, 14);
    assert_eq!(r[14], Some(50.0), "no gains and no losses reads as neutral");
    assert_eq!(*r.last().unwrap(), Some(50.0));

    let falling: Vec<f64> = (0..30).map(|i| 100.0 - f64::from(i)).collect();
    assert_eq!(*indicators::rsi(&falling, 14).last().unwrap(), Some(0.0));
}

#[test]
fn macd_line_signal_and_histogram_match_reference() {
    let (line, signal, hist) = indicators::macd(CLOSES, 3, 6, 3);
    assert_series(&line, MACD_LINE);
    assert_series(&signal, MACD_SIGNAL);
    assert_series(&hist, MACD_HIST);
    assert_eq!(
        line.iter().position(Option::is_some),
        Some(5),
        "starts at slow - 1"
    );
    assert_eq!(
        signal.iter().position(Option::is_some),
        Some(7),
        "starts at slow + signal - 2"
    );
}

#[test]
fn macd_default_periods_warm_up_after_34_bars() {
    let closes: Vec<f64> = (0..60)
        .map(|i| 100.0 + (f64::from(i) * 0.3).sin())
        .collect();
    let (line, signal, hist) = indicators::macd(&closes, 12, 26, 9);
    assert_eq!(line.iter().position(Option::is_some), Some(25));
    assert_eq!(signal.iter().position(Option::is_some), Some(33));
    assert_eq!(hist.iter().position(Option::is_some), Some(33));
}

#[test]
fn bollinger_uses_population_deviation() {
    let (mid, upper, lower) = indicators::bollinger(CLOSES, 5, 2.0);
    assert_series(&mid, BOLL_MID);
    assert_series(&upper, BOLL_UPPER);
    assert_series(&lower, BOLL_LOWER);
}

#[test]
fn bollinger_of_a_flat_series_has_zero_width() {
    let (mid, upper, lower) = indicators::bollinger(&[7.0; 10], 4, 2.0);
    assert_eq!(mid[9], Some(7.0));
    assert_eq!(upper[9], Some(7.0));
    assert_eq!(lower[9], Some(7.0));
}

fn gap_bars() -> Vec<Bar> {
    bars_from(&[
        (10.0, 11.0, 9.0, 10.0),
        (10.0, 12.0, 10.0, 11.0),
        (11.0, 12.0, 10.5, 11.5),
        (14.0, 15.0, 13.5, 14.5), // gaps up: the range from the prior close counts
        (14.5, 15.0, 14.0, 14.2),
        (14.2, 14.5, 13.0, 13.5),
        (13.5, 14.0, 13.2, 13.8),
        (13.8, 14.9, 13.7, 14.8),
    ])
}

#[test]
fn true_range_includes_gaps() {
    let tr = indicators::true_range(&gap_bars());
    let expected = [
        None,
        Some(2.0),
        Some(1.5),
        Some(3.5),
        Some(1.0),
        Some(1.5),
        Some(0.8),
        Some(1.2),
    ];
    assert_series(&tr, &expected);
}

#[test]
fn atr_is_wilder_smoothed_over_true_range() {
    let a = indicators::atr(&gap_bars(), 3);
    let expected = [
        None,
        None,
        None,
        Some(2.333_333_333_333),
        Some(1.888_888_888_889),
        Some(1.759_259_259_259),
        Some(1.439_506_172_84),
        Some(1.359_670_781_893),
    ];
    assert_series(&a, &expected);
}

#[test]
fn vwap_weights_typical_price_by_volume() {
    let bars = [
        bar(10.0, 12.0, 9.0, 11.0, 100),
        bar(11.0, 13.0, 10.0, 12.0, 300),
        bar(12.0, 12.6, 11.4, 12.3, 0),
    ];
    // typical prices 10.6667, 11.6667, 12.1 -> (10.6667 * 100 + 11.6667 * 300 + 0) / 400
    let v = indicators::vwap(&bars).unwrap();
    assert!((v - 11.416_666_666_666_664).abs() < EPS, "{v}");
}

#[test]
fn vwap_with_no_volume_is_the_plain_mean_of_typical_prices() {
    let bars = [
        bar(10.0, 12.0, 9.0, 11.0, 0),
        bar(11.0, 13.0, 10.0, 12.0, 0),
        bar(12.0, 12.6, 11.4, 12.3, 0),
    ];
    let v = indicators::vwap(&bars).unwrap();
    assert!((v - 11.477_777_777_777_776).abs() < EPS, "{v}");
}

#[test]
fn classic_pivots() {
    let prev = bar(1940.0, 1950.0, 1930.0, 1942.0, 1);
    let Pivots {
        pivot,
        r1,
        r2,
        s1,
        s2,
    } = indicators::pivots(&prev);
    assert!((pivot - 1_940.666_666_666_666_7).abs() < EPS);
    assert!((r1 - 1_951.333_333_333_333_5).abs() < EPS);
    assert!((r2 - 1_960.666_666_666_666_7).abs() < EPS);
    assert!((s1 - 1_931.333_333_333_333_5).abs() < EPS);
    assert!((s2 - 1_920.666_666_666_666_7).abs() < EPS);
}

#[test]
fn short_empty_and_zero_period_inputs_give_all_none() {
    let none = |v: &[Option<f64>]| v.iter().all(Option::is_none);
    assert!(indicators::sma(&[], 5).is_empty());
    assert!(none(&indicators::sma(&[1.0, 2.0], 5)));
    assert!(none(&indicators::sma(&[1.0, 2.0], 0)));
    assert!(none(&indicators::ema(&[1.0, 2.0, 3.0], 4)));
    assert!(none(&indicators::ema(&[1.0, 2.0, 3.0], 0)));
    assert!(
        none(&indicators::rsi(&[1.0; 14], 14)),
        "needs period + 1 closes"
    );
    assert!(none(&indicators::rsi(&[1.0; 5], 0)));
    let (a, b, c) = indicators::macd(&[1.0; 10], 12, 26, 9);
    assert!(none(&a) && none(&b) && none(&c));
    assert!(
        none(&indicators::atr(&gap_bars()[..3], 3)),
        "needs period + 1 bars"
    );
    assert!(none(&indicators::atr(&[], 14)));
    let (m, u, l) = indicators::bollinger(&[1.0, 2.0], 20, 2.0);
    assert!(none(&m) && none(&u) && none(&l));
    assert_eq!(indicators::vwap(&[]), None);
    assert_eq!(indicators::swing_levels(&[], 3, 3), (vec![], vec![]));
}

#[test]
fn nan_input_never_panics_and_poisons_only_what_depends_on_it() {
    let mut v = CLOSES.to_vec();
    v[10] = f64::NAN;
    let s = indicators::sma(&v, 5);
    assert!(s[9].is_some() && s[10..15].iter().all(Option::is_none) && s[15].is_some());
    let e = indicators::ema(&v, 5);
    assert!(e[9].is_some() && e[10..].iter().all(Option::is_none));
    let r = indicators::rsi(&v, 14);
    assert!(r.iter().all(Option::is_none));
    let _ = indicators::macd(&v, 3, 6, 3);
    let _ = indicators::bollinger(&v, 5, 2.0);
    let mut bars = gap_bars();
    bars[4].high = f64::NAN;
    let a = indicators::atr(&bars, 3);
    assert!(a[3].is_some() && a[4].is_none() && a[7].is_none());
    bars[1].close = f64::NAN;
    assert!(indicators::atr(&bars, 3).iter().all(Option::is_none));
    assert_eq!(indicators::vwap(&bars), None);
    let _ = indicators::swing_levels(&bars, 1, 1);
}

fn swing_bars(highs: &[f64]) -> Vec<Bar> {
    // Lows sit 2 below the highs so the fractal lows mirror the fractal highs.
    let rows: Vec<_> = highs
        .iter()
        .map(|h| (h - 1.0, *h, h - 2.0, h - 1.0))
        .collect();
    bars_from(&rows)
}

const SWING_HIGHS: [f64; 15] = [
    10.0, 11.0, 12.0, 15.0, 12.0, 11.0, 10.0, 11.0, 12.0, 15.1, 12.0, 11.0, 10.0, 9.0, 8.0,
];

#[test]
fn swing_levels_find_fractals_and_sort_by_distance() {
    let bars = swing_bars(&SWING_HIGHS);
    // Without merging: resistances 15.0 and 15.1, nearest to the reference first.
    // The only fractal low is index 6 (high 10, low 8).
    let (sup, res) = indicators::swing_levels_around(&bars, 3, 3, 0.0, 12.5);
    assert_eq!(res, vec![15.0, 15.1]);
    assert_eq!(sup, vec![8.0]);
    // Within the tolerance the two highs merge into their mean.
    let (_, res) = indicators::swing_levels_around(&bars, 3, 3, 0.2, 12.5);
    assert_eq!(res.len(), 1);
    assert!((res[0] - 15.05).abs() < EPS);
    // A reference above both highs: no resistance left, the support stays.
    let (sup, res) = indicators::swing_levels_around(&bars, 3, 3, 0.0, 20.0);
    assert!(res.is_empty());
    assert_eq!(sup, vec![8.0]);
}

#[test]
fn swing_levels_count_a_flat_top_once() {
    let bars = swing_bars(&[1.0, 2.0, 3.0, 5.0, 5.0, 3.0, 2.0, 1.0]);
    let (_, res) = indicators::swing_levels_around(&bars, 2, 2, 0.0, 3.0);
    assert_eq!(res, vec![5.0]);
}

#[test]
fn swing_levels_default_tolerance_is_a_quarter_atr() {
    let bars = swing_bars(&SWING_HIGHS);
    let last = bars.last().unwrap();
    let atr = indicators::atr(&bars, 14)
        .last()
        .copied()
        .flatten()
        .unwrap();
    assert_eq!(
        indicators::swing_levels(&bars, 3, 3),
        indicators::swing_levels_around(&bars, 3, 3, 0.25 * atr, last.close)
    );
}

#[test]
fn swing_levels_need_enough_bars_on_both_sides() {
    let bars = swing_bars(&SWING_HIGHS[..5]);
    assert_eq!(
        indicators::swing_levels_around(&bars, 3, 3, 0.0, 9.0),
        (vec![], vec![])
    );
    assert_eq!(
        indicators::swing_levels_around(&bars, 0, 3, 0.0, 9.0),
        (vec![], vec![])
    );
}
