//! Golden tests for `analyze` on deterministic synthetic markets.

mod common;

use common::*;
use jarvis_market::indicators;
use jarvis_market::{
    Bar, Bias, Importance, MarketView, NewsInput, Side, Timeframe, Trend, ViewConfig, ViewInput,
    analyze,
};

fn cfg() -> ViewConfig {
    ViewConfig::default()
}

fn run(input: &ViewInput) -> MarketView {
    analyze(input, &cfg())
}

/// The frame without its forming last bar (every frame in these inputs is forming at NOW).
fn closed(input: &ViewInput, tf: Timeframe) -> Vec<Bar> {
    let mut bars = input.frames[&tf].clone();
    bars.pop();
    bars
}

fn last(series: Vec<Option<f64>>) -> f64 {
    series
        .last()
        .copied()
        .flatten()
        .expect("series has a value")
}

fn points(view: &MarketView, name: &str) -> f64 {
    view.factors
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("factor {name} missing in {:?}", view.factors))
        .points
}

fn has_factor(view: &MarketView, name: &str) -> bool {
    view.factors.iter().any(|f| f.name == name)
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// --- directions -------------------------------------------------------------------

#[test]
fn uptrend_is_bullish_with_a_buy_setup() {
    let input = uptrend_input();
    let view = run(&input);

    assert_eq!(view.symbol, "XAUUSD");
    assert_eq!(view.bias, Bias::Bullish);
    assert!(!view.stand_aside);
    assert_eq!(view.stand_aside_reason, None);
    let setup = view.setup.as_ref().expect("a buy setup");
    assert_eq!(setup.side, Side::Buy);

    assert_eq!(points(&view, "H4 trend"), 25.0);
    assert_eq!(points(&view, "H1 trend"), 20.0);
    assert_eq!(points(&view, "VWAP"), 10.0);
    assert_eq!(points(&view, "Daily pivot"), 10.0);
    assert!(view.score >= 20.0);
    assert_eq!(view.confidence, view.score.round() as u32);
    assert!(!has_factor(&view, "News"), "no news input, no news factor");
    assert_eq!(view.news.label, "not configured");
    assert!(view.notes.is_empty(), "{:?}", view.notes);

    for row in &view.timeframes {
        assert_eq!(row.trend, Trend::Up, "{:?}", row.timeframe);
    }
    let order: Vec<_> = view.timeframes.iter().map(|r| r.timeframe).collect();
    assert_eq!(order, [Timeframe::H4, Timeframe::H1, Timeframe::M15]);
}

#[test]
fn downtrend_is_bearish_with_a_sell_setup() {
    let view = run(&downtrend_input());

    assert_eq!(view.bias, Bias::Bearish);
    let setup = view.setup.as_ref().expect("a sell setup");
    assert_eq!(setup.side, Side::Sell);
    assert_eq!(points(&view, "H4 trend"), -25.0);
    assert_eq!(points(&view, "H1 trend"), -20.0);
    assert_eq!(points(&view, "VWAP"), -10.0);
    assert_eq!(points(&view, "Daily pivot"), -10.0);
    assert!(view.score <= -20.0);
    assert_eq!(view.confidence, view.score.abs().round() as u32);
    // A one-way fall leaves RSI deeply oversold, which counts for a bounce.
    assert!(points(&view, "M15 RSI") > 0.0);
    let rsi_factor = view.factors.iter().find(|f| f.name == "M15 RSI").unwrap();
    assert!(
        rsi_factor.detail.contains("oversold"),
        "{}",
        rsi_factor.detail
    );
}

#[test]
fn a_range_is_neutral_without_a_setup() {
    let view = run(&range_input());
    assert_eq!(view.bias, Bias::Neutral);
    assert!(view.score.abs() < 20.0, "{}", view.score);
    assert_eq!(view.setup, None);
    assert!(!view.stand_aside);
    assert_eq!(points(&view, "H4 trend"), 0.0);
    assert_eq!(points(&view, "H1 trend"), 0.0);
    assert_eq!(view.confidence, view.score.abs().round() as u32);
    assert!(
        view.summary.starts_with("Neutral intraday bias"),
        "{}",
        view.summary
    );
    assert!(
        view.summary.contains("No setup is offered"),
        "{}",
        view.summary
    );
}

#[test]
fn momentum_and_rsi_follow_the_scoring_table() {
    for input in [
        uptrend_input(),
        downtrend_input(),
        range_input(),
        pullback_input(),
    ] {
        let view = run(&input);
        let closes: Vec<f64> = closed(&input, Timeframe::M15)
            .iter()
            .map(|b| b.close)
            .collect();

        // RSI(14) bands: >70 -1, 55..70 +0.5, 45..55 0, 30..45 -0.5, <30 +1.
        let rsi = last(indicators::rsi(&closes, 14));
        let band = if rsi > 70.0 {
            -1.0
        } else if rsi >= 55.0 {
            0.5
        } else if rsi >= 45.0 {
            0.0
        } else if rsi >= 30.0 {
            -0.5
        } else {
            1.0
        };
        assert_eq!(points(&view, "M15 RSI"), band * 10.0, "rsi {rsi}");

        // MACD histogram sign: half weight, another half when it grew over 3 bars.
        let (_, _, hist) = indicators::macd(&closes, 12, 26, 9);
        let h = hist[hist.len() - 1].unwrap();
        let before = hist[hist.len() - 4].unwrap();
        let grew = (h > 0.0 && h > before) || (h < 0.0 && h < before);
        let expected = h.signum() * if grew { 1.0 } else { 0.5 } * 15.0;
        assert_eq!(
            points(&view, "M15 momentum"),
            expected,
            "hist {h} before {before}"
        );
    }
}

// --- the forming bar and the clock -------------------------------------------------

#[test]
fn the_forming_bar_is_ignored() {
    let input = uptrend_input();
    let baseline = run(&input);
    let mut tampered = input.clone();
    for bars in tampered.frames.values_mut() {
        let b = bars.last_mut().unwrap();
        b.open = 1.0;
        b.high = 1.0e6;
        b.low = 1.0;
        b.close = 1.0e6;
    }
    assert_eq!(run(&tampered), baseline);
}

#[test]
fn a_finished_last_bar_is_used() {
    // A week later the market has been closed: the last bar of each frame has ended.
    let mut input = uptrend_input();
    input.now += 7 * 86_400;
    let view = run(&input);
    let h1_all = &input.frames[&Timeframe::H1];
    let atr = last(indicators::atr(h1_all, 14));
    let row = view
        .timeframes
        .iter()
        .find(|r| r.timeframe == Timeframe::H1)
        .unwrap();
    assert!(near(row.atr, atr, 0.0006), "{} vs {atr}", row.atr);
}

#[test]
fn vwap_covers_only_the_current_server_day() {
    let input = uptrend_input();
    let view = run(&input);
    let today: Vec<Bar> = closed(&input, Timeframe::M5)
        .into_iter()
        .filter(|b| b.time >= DAY0)
        .collect();
    assert_eq!(today.len(), 145, "00:00 to 12:05 in five-minute bars");
    let expected = indicators::vwap(&today).unwrap();
    assert!(near(view.levels.vwap.unwrap(), expected, 0.0006));
    let all = closed(&input, Timeframe::M5);
    let across_days = indicators::vwap(&all).unwrap();
    assert!(
        !near(across_days, expected, 1.0),
        "the fixture must tell the days apart"
    );
}

#[test]
fn right_after_midnight_there_is_no_session_vwap() {
    let now = DAY0 + 180;
    let mut input = input_from(&uptrend_path, 1.0, 0.2);
    input.now = now;
    input.frames = frames(now, 1.0, &uptrend_path);
    let view = run(&input);
    assert_eq!(view.levels.vwap, None);
    assert!(!has_factor(&view, "VWAP"));
    assert!(
        view.notes.iter().any(|n| n.contains("VWAP")),
        "{:?}",
        view.notes
    );
    assert!(
        has_factor(&view, "H4 trend"),
        "everything else is still computed"
    );
}

#[test]
fn pivots_come_from_the_previous_completed_day() {
    let input = uptrend_input();
    let view = run(&input);
    let d1 = closed(&input, Timeframe::D1);
    let prev = d1.last().unwrap();
    assert_eq!(prev.time, DAY0 - 86_400);
    let p = indicators::pivots(prev);
    assert!(near(view.levels.pivot.unwrap(), p.pivot, 0.0006));
    assert!(near(view.levels.r1.unwrap(), p.r1, 0.0006));
    assert!(near(view.levels.s2.unwrap(), p.s2, 0.0006));
    assert!(near(view.levels.prev_day_high.unwrap(), prev.high, 0.0006));
    assert!(near(view.levels.prev_day_low.unwrap(), prev.low, 0.0006));
}

// --- stand aside -------------------------------------------------------------------

#[test]
fn a_high_impact_event_close_by_means_stand_aside() {
    let mut input = uptrend_input();
    input.events = vec![event(10, "USD", "Nonfarm Payrolls", Importance::High)];
    let view = run(&input);

    assert_eq!(view.bias, Bias::Bullish, "the bias is still reported");
    assert!(view.stand_aside);
    let reason = view.stand_aside_reason.as_deref().unwrap();
    assert!(
        reason.contains("Nonfarm Payrolls") && reason.contains("in 10 minutes"),
        "{reason}"
    );
    assert_eq!(view.setup, None);
    assert_eq!(view.events.len(), 1);
    assert_eq!(view.events[0].minutes_away, 10);
    assert_eq!(view.events[0].importance, Importance::High);
    assert!(view.summary.contains("Standing aside"), "{}", view.summary);
    assert!(
        view.summary.contains("No setup is offered"),
        "{}",
        view.summary
    );
}

#[test]
fn only_high_impact_events_for_the_instruments_currencies_count() {
    let stands_aside = |events: Vec<jarvis_market::EventInput>| {
        let mut input = uptrend_input();
        input.events = events;
        run(&input).stand_aside
    };
    assert!(
        stands_aside(vec![event(30, "USD", "CPI", Importance::High)]),
        "30 minutes is inside"
    );
    assert!(!stands_aside(vec![event(
        31,
        "USD",
        "CPI",
        Importance::High
    )]));
    assert!(
        stands_aside(vec![event(-30, "USD", "CPI", Importance::High)]),
        "just after counts too"
    );
    assert!(!stands_aside(vec![event(
        -31,
        "USD",
        "CPI",
        Importance::High
    )]));
    assert!(!stands_aside(vec![event(
        10,
        "USD",
        "CPI",
        Importance::Moderate
    )]));
    assert!(!stands_aside(vec![event(
        10,
        "EUR",
        "ECB rate decision",
        Importance::High
    )]));
    assert!(
        stands_aside(vec![event(10, "usd", "CPI", Importance::High)]),
        "currency match ignores case"
    );
    // Past events read "ago".
    let mut input = uptrend_input();
    input.events = vec![event(-12, "USD", "ISM", Importance::High)];
    let view = run(&input);
    assert!(view.stand_aside_reason.unwrap().contains("12 minutes ago"));
}

#[test]
fn a_wide_spread_means_stand_aside() {
    let input = uptrend_input();
    let atr_m15 = last(indicators::atr(&closed(&input, Timeframe::M15), 14));

    let mut tight = input.clone();
    tight.ask = round2(tight.bid + 0.2 * atr_m15);
    assert!(!run(&tight).stand_aside);
    assert!(run(&tight).setup.is_some());

    let mut wide = input.clone();
    wide.ask = round2(wide.bid + 0.3 * atr_m15);
    let view = run(&wide);
    assert!(view.stand_aside);
    let reason = view.stand_aside_reason.as_deref().unwrap();
    assert!(
        reason.contains("Spread") && reason.contains("25%") && reason.contains("M15 ATR"),
        "{reason}"
    );
    assert!(reason.contains("points"), "uses the point size: {reason}");
    assert_eq!(view.setup, None);
    assert_eq!(view.bias, Bias::Bullish);
}

#[test]
fn both_reasons_are_reported_together() {
    let mut input = uptrend_input();
    input.ask = input.bid + 5.0;
    input.events = vec![event(5, "USD", "FOMC", Importance::High)];
    let reason = run(&input).stand_aside_reason.unwrap();
    assert!(
        reason.contains("FOMC") && reason.contains("Spread"),
        "{reason}"
    );
}

// --- setup arithmetic --------------------------------------------------------------

#[test]
fn buy_setup_follows_the_atr_rules() {
    let input = uptrend_input();
    let a = last(indicators::atr(&closed(&input, Timeframe::H1), 14));
    let m = last(indicators::atr(&closed(&input, Timeframe::M15), 14));
    let setup = run(&input).setup.unwrap();

    let low = input.ask - 0.5 * m;
    let mid = (low + input.ask) / 2.0;
    assert_eq!(setup.entry_high, input.ask);
    assert_eq!(setup.entry_low, round2(low));
    assert_eq!(setup.stop_loss, round2(mid - a));
    assert_eq!(setup.take_profit, round2(mid + 1.5 * a));
    assert_eq!(
        setup.take_profit_2,
        round2(mid + 2.5 * a),
        "no swing resistance above"
    );
    assert_eq!(setup.risk_reward, 1.5);
    assert!(near(setup.atr_h1, a, 0.0006) && near(setup.atr_m15, m, 0.0006));
    assert!(setup.stop_loss < setup.entry_low && setup.take_profit_2 > setup.take_profit);
}

#[test]
fn sell_setup_mirrors_it_with_the_bid() {
    let input = downtrend_input();
    let a = last(indicators::atr(&closed(&input, Timeframe::H1), 14));
    let m = last(indicators::atr(&closed(&input, Timeframe::M15), 14));
    let setup = run(&input).setup.unwrap();

    let high = input.bid + 0.5 * m;
    let mid = (input.bid + high) / 2.0;
    assert_eq!(setup.side, Side::Sell);
    assert_eq!(setup.entry_low, input.bid);
    assert_eq!(setup.entry_high, round2(high));
    assert_eq!(setup.stop_loss, round2(mid + a));
    assert_eq!(setup.take_profit, round2(mid - 1.5 * a));
    assert_eq!(setup.take_profit_2, round2(mid - 2.5 * a));
    assert_eq!(setup.risk_reward, 1.5);
    assert!(setup.take_profit_2 < setup.take_profit && setup.take_profit < setup.entry_low);
}

#[test]
fn the_second_target_snaps_to_a_nearby_swing_resistance() {
    let input = pullback_input();
    let h1 = closed(&input, Timeframe::H1);
    let a = last(indicators::atr(&h1, 14));
    let m = last(indicators::atr(&closed(&input, Timeframe::M15), 14));
    let mid = (input.ask - 0.5 * m + input.ask) / 2.0;
    let tp1 = mid + 1.5 * a;
    let (_, resistances) =
        indicators::swing_levels_around(&h1, 3, 3, 0.25 * a, (input.bid + input.ask) / 2.0);
    let expected = resistances
        .iter()
        .copied()
        .filter(|r| *r > tp1 && *r - mid <= 3.0 * a)
        .fold(f64::INFINITY, f64::min);
    assert!(
        expected.is_finite(),
        "fixture must have a resistance between TP1 and 3 ATR: {resistances:?}"
    );

    let view = run(&input);
    let setup = view.setup.unwrap();
    assert_eq!(setup.take_profit_2, round2(expected));
    assert_ne!(
        setup.take_profit_2,
        round2(mid + 2.5 * a),
        "not the fallback"
    );
    assert!(
        view.levels
            .resistance
            .iter()
            .any(|r| near(*r, expected, 0.0006))
    );

    // A resistance further away than the search distance is ignored.
    let mut narrow = cfg();
    narrow.tp2_search_atr_h1 = 0.5;
    let setup = analyze(&input, &narrow).setup.unwrap();
    assert_eq!(setup.take_profit_2, round2(mid + 2.5 * a));
}

#[test]
fn config_changes_the_setup_multipliers() {
    let input = uptrend_input();
    let a = last(indicators::atr(&closed(&input, Timeframe::H1), 14));
    let m = last(indicators::atr(&closed(&input, Timeframe::M15), 14));
    let mut c = cfg();
    c.stop_atr_h1 = 2.0;
    c.tp1_atr_h1 = 3.0;
    c.tp2_fallback_atr_h1 = 4.0;
    c.entry_zone_atr_m15 = 1.0;
    let setup = analyze(&input, &c).setup.unwrap();
    let low = input.ask - m;
    let mid = (low + input.ask) / 2.0;
    assert_eq!(setup.entry_low, round2(low));
    assert_eq!(setup.stop_loss, round2(mid - 2.0 * a));
    assert_eq!(setup.take_profit, round2(mid + 3.0 * a));
    assert_eq!(setup.risk_reward, 1.5);
    c.tp1_atr_h1 = 4.0;
    assert_eq!(analyze(&input, &c).setup.unwrap().risk_reward, 2.0);
}

#[test]
fn valid_until_is_four_hours_after_now() {
    let view = run(&uptrend_input());
    assert_eq!(view.valid_until, NOW + 240 * 60);
    let mut c = cfg();
    c.horizon_minutes = 60;
    assert_eq!(analyze(&uptrend_input(), &c).valid_until, NOW + 3600);
}

// --- bias, confidence --------------------------------------------------------------

#[test]
fn bias_thresholds_are_inclusive() {
    let up = uptrend_input();
    let score = run(&up).score;
    let mut c = cfg();
    c.bias_threshold = score;
    assert_eq!(
        analyze(&up, &c).bias,
        Bias::Bullish,
        "score at the threshold"
    );
    c.bias_threshold = score + 0.5;
    assert_eq!(analyze(&up, &c).bias, Bias::Neutral);

    let down = downtrend_input();
    let score = run(&down).score;
    c.bias_threshold = -score;
    assert_eq!(analyze(&down, &c).bias, Bias::Bearish);
    c.bias_threshold = -score + 0.5;
    assert_eq!(analyze(&down, &c).bias, Bias::Neutral);
}

/// H4 trends one way and everything shorter the other.
fn disagreeing_input() -> ViewInput {
    let mut input = downtrend_input();
    let up = uptrend_input();
    input
        .frames
        .insert(Timeframe::H4, up.frames[&Timeframe::H4].clone());
    input
}

#[test]
fn confidence_is_capped_when_h4_and_h1_disagree() {
    // Weights chosen so the score is large and the cap decides the confidence.
    let mut c = cfg();
    c.weight_h4_trend = 5.0;
    c.weight_h1_trend = 90.0;

    let view = analyze(&disagreeing_input(), &c);
    assert_eq!(points(&view, "H4 trend"), 5.0);
    assert_eq!(points(&view, "H1 trend"), -90.0);
    assert_eq!(view.bias, Bias::Bearish);
    assert!(view.score < -50.0, "{}", view.score);
    assert_eq!(view.confidence, 50);

    // The same strength without a disagreement is not capped.
    let agreeing = analyze(&downtrend_input(), &c);
    assert_eq!(points(&agreeing, "H4 trend"), -5.0);
    assert!(agreeing.confidence > 50);
    assert_eq!(
        agreeing.confidence,
        agreeing.score.abs().round().min(100.0) as u32
    );
}

#[test]
fn a_mixed_trend_is_not_a_disagreement() {
    let mut c = cfg();
    c.weight_h1_trend = 90.0;
    let mut input = uptrend_input();
    input
        .frames
        .insert(Timeframe::H4, range_input().frames[&Timeframe::H4].clone());
    let view = analyze(&input, &c);
    assert_eq!(points(&view, "H4 trend"), 0.0);
    assert!(view.confidence > 50, "{}", view.confidence);
}

#[test]
fn score_is_clamped_to_100() {
    let mut c = cfg();
    c.weight_h4_trend = 80.0;
    c.weight_h1_trend = 80.0;
    let view = analyze(&downtrend_input(), &c);
    assert_eq!(view.score, -100.0);
    assert_eq!(view.confidence, 100);
}

// --- news --------------------------------------------------------------------------

#[test]
fn news_adds_a_factor_when_present() {
    let mut input = uptrend_input();
    let without = run(&input);
    input.news = Some(NewsInput {
        score: 0.6,
        count: 7,
    });
    let view = run(&input);
    assert_eq!(points(&view, "News"), 6.0);
    assert!(near(view.score, without.score + 6.0, 1e-9));
    assert!(view.news.configured);
    assert_eq!(view.news.score, 0.6);
    assert_eq!(view.news.label, "positive");
    assert_eq!(view.news.count, 7);

    input.news = Some(NewsInput {
        score: -0.2,
        count: 3,
    });
    let view = run(&input);
    assert_eq!(points(&view, "News"), -2.0);
    assert_eq!(view.news.label, "slightly negative");

    input.news = Some(NewsInput {
        score: 4.0,
        count: 1,
    });
    assert_eq!(points(&run(&input), "News"), 10.0, "clamped to the weight");

    input.news = Some(NewsInput {
        score: f64::NAN,
        count: 1,
    });
    let view = run(&input);
    assert!(!has_factor(&view, "News"));
    assert!(
        view.notes.iter().any(|n| n.contains("News")),
        "{:?}",
        view.notes
    );
}

#[test]
fn news_alone_cannot_make_a_view() {
    let mut input = ViewInput {
        symbol: "XAUUSD".into(),
        digits: 2,
        point: 0.01,
        bid: 3900.0,
        ask: 3900.2,
        now: NOW,
        ..ViewInput::default()
    };
    input.news = Some(NewsInput {
        score: 1.0,
        count: 20,
    });
    let view = run(&input);
    assert_eq!(view.bias, Bias::Neutral);
    assert!(view.factors.is_empty());
    assert_eq!(view.setup, None);
}

// --- events ------------------------------------------------------------------------

#[test]
fn events_are_windowed_sorted_and_timed_from_now() {
    let mut input = uptrend_input();
    input.events = vec![
        event(23 * 60, "EUR", "late", Importance::Low),
        event(25 * 60, "USD", "too far ahead", Importance::High),
        event(-59, "USD", "recent", Importance::Moderate),
        event(-61, "USD", "too old", Importance::High),
        event(90, "USD", "soon", Importance::High),
        event(90, "EUR", "same time", Importance::Moderate),
    ];
    let view = run(&input);
    let titles: Vec<_> = view.events.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["recent", "same time", "soon", "late"]);
    let minutes: Vec<_> = view.events.iter().map(|e| e.minutes_away).collect();
    assert_eq!(minutes, [-59, 90, 90, 23 * 60]);
    assert_eq!(view.events[0].time, NOW - 59 * 60);
    assert!(
        !view.stand_aside,
        "the 90 minute event is outside the blackout"
    );
}

// --- missing data ------------------------------------------------------------------

#[test]
fn missing_inputs_drop_their_factor_and_are_listed() {
    let mut input = uptrend_input();
    input.frames.remove(&Timeframe::D1);
    input.frames.get_mut(&Timeframe::H4).unwrap().truncate(40);
    let view = run(&input);
    assert!(!has_factor(&view, "Daily pivot") && !has_factor(&view, "H4 trend"));
    assert!(has_factor(&view, "H1 trend") && has_factor(&view, "VWAP"));
    assert_eq!(view.levels.pivot, None);
    assert_eq!(view.levels.prev_day_high, None);
    assert!(
        view.notes.iter().any(|n| n.contains("Daily pivot")),
        "{:?}",
        view.notes
    );
    assert!(
        view.notes
            .iter()
            .any(|n| n.contains("H4 trend") && n.contains("50")),
        "{:?}",
        view.notes
    );
    assert!(view.timeframes.iter().all(|r| r.timeframe != Timeframe::H4));
    // The score is the plain sum of the factors that remain.
    let sum: f64 = view.factors.iter().map(|f| f.points).sum();
    assert!(near(view.score, sum, 1e-9));
}

#[test]
fn bars_needed_for_every_factor() {
    // 51 supplied bars (50 closed) are enough for the trend factors; 35 for momentum.
    let mut input = uptrend_input();
    for (tf, n) in [
        (Timeframe::H4, 51),
        (Timeframe::H1, 51),
        (Timeframe::M15, 51),
    ] {
        let bars = input.frames.get_mut(&tf).unwrap();
        let keep = bars.len() - n;
        bars.drain(..keep);
    }
    let view = run(&input);
    assert_eq!(view.factors.len(), 6, "{:?}", view.notes);
    assert!(view.notes.is_empty(), "{:?}", view.notes);

    let m15 = input.frames.get_mut(&Timeframe::M15).unwrap();
    let keep = m15.len() - 34; // 33 closed bars: one short for the MACD histogram
    m15.drain(..keep);
    let view = run(&input);
    assert!(!has_factor(&view, "M15 momentum") && has_factor(&view, "M15 RSI"));
}

#[test]
fn too_little_data_gives_a_neutral_view_and_a_note() {
    let mut input = uptrend_input();
    for bars in input.frames.values_mut() {
        let keep = bars.len() - 8;
        bars.drain(..keep);
    }
    let view = run(&input);
    assert_eq!(view.bias, Bias::Neutral);
    assert_eq!(view.score, 0.0);
    assert_eq!(view.confidence, 0);
    assert_eq!(view.setup, None);
    assert!(view.factors.is_empty());
    assert!(!view.notes.is_empty());
    assert!(
        view.summary.starts_with("Not enough price data"),
        "{}",
        view.summary
    );

    let empty = ViewInput {
        bid: 1.0,
        ask: 1.1,
        now: NOW,
        ..ViewInput::default()
    };
    let view = run(&empty);
    assert_eq!(view.bias, Bias::Neutral);
    assert!(view.setup.is_none() && !view.notes.is_empty());
    assert_eq!(view.disclaimer, jarvis_market::DISCLAIMER);
}

#[test]
fn an_invalid_price_gives_a_neutral_view() {
    for (bid, ask) in [(0.0, 0.0), (f64::NAN, 1.0), (10.0, 9.0), (-1.0, 1.0)] {
        let mut input = uptrend_input();
        input.bid = bid;
        input.ask = ask;
        let view = run(&input);
        assert_eq!(view.bias, Bias::Neutral, "{bid} {ask}");
        assert_eq!(view.setup, None);
        assert!(view.price.bid.is_finite() && view.price.ask.is_finite());
        assert!(view.notes.iter().any(|n| n.contains("bid/ask")));
    }
}

// --- determinism and shape ---------------------------------------------------------

#[test]
fn analyze_is_deterministic() {
    for input in [
        uptrend_input(),
        downtrend_input(),
        range_input(),
        pullback_input(),
    ] {
        let first = run(&input);
        let second = run(&input);
        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
    }
}

#[test]
fn json_shape_matches_the_documented_dto() {
    let mut input = uptrend_input();
    input.events = vec![event(120, "USD", "Nonfarm Payrolls", Importance::High)];
    input.news = Some(NewsInput {
        score: 0.2,
        count: 7,
    });
    let view = run(&input);
    let json = serde_json::to_value(&view).unwrap();

    let keys = |v: &serde_json::Value| -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(
        keys(&json),
        [
            "bias",
            "confidence",
            "disclaimer",
            "events",
            "factors",
            "levels",
            "news",
            "notes",
            "price",
            "score",
            "setup",
            "stand_aside",
            "stand_aside_reason",
            "summary",
            "symbol",
            "timeframes",
            "valid_until"
        ]
    );
    assert_eq!(json["symbol"], "XAUUSD");
    assert_eq!(json["bias"], "bullish");
    assert_eq!(json["stand_aside"], false);
    assert!(json["stand_aside_reason"].is_null());
    assert_eq!(json["valid_until"], NOW + 14_400);
    assert_eq!(json["disclaimer"], jarvis_market::DISCLAIMER);
    assert_eq!(keys(&json["price"]), ["ask", "bid"]);
    assert_eq!(
        keys(&json["setup"]),
        [
            "atr_h1",
            "atr_m15",
            "entry_high",
            "entry_low",
            "risk_reward",
            "side",
            "stop_loss",
            "take_profit",
            "take_profit_2"
        ]
    );
    assert_eq!(json["setup"]["side"], "buy");
    assert_eq!(
        keys(&json["timeframes"][0]),
        [
            "atr",
            "ema20",
            "ema50",
            "macd_hist",
            "rsi",
            "timeframe",
            "trend"
        ]
    );
    assert_eq!(json["timeframes"][0]["timeframe"], "H4");
    assert_eq!(json["timeframes"][0]["trend"], "up");
    assert_eq!(
        keys(&json["levels"]),
        [
            "pivot",
            "prev_day_high",
            "prev_day_low",
            "r1",
            "r2",
            "resistance",
            "s1",
            "s2",
            "support",
            "vwap"
        ]
    );
    assert_eq!(
        keys(&json["factors"][0]),
        ["detail", "name", "points", "signal"]
    );
    assert_eq!(json["factors"][0]["name"], "H4 trend");
    assert_eq!(json["factors"][0]["signal"], "bullish");
    assert_eq!(json["factors"][0]["points"], 25.0);
    assert_eq!(
        json["factors"][0]["detail"],
        "EMA20 above EMA50, close above EMA50"
    );
    assert_eq!(
        keys(&json["events"][0]),
        ["currency", "importance", "minutes_away", "time", "title"]
    );
    assert_eq!(json["events"][0]["importance"], "high");
    assert_eq!(json["events"][0]["minutes_away"], 120);
    assert_eq!(
        keys(&json["news"]),
        ["configured", "count", "label", "score"]
    );
    assert_eq!(json["news"]["label"], "slightly positive");
    assert_eq!(json["notes"], serde_json::json!([]));

    // And it reads back unchanged.
    let back: MarketView = serde_json::from_value(json).unwrap();
    assert_eq!(back, view);
}

#[test]
fn no_setup_and_stand_aside_serialise_as_null_and_a_reason() {
    let mut input = uptrend_input();
    input.events = vec![event(5, "USD", "FOMC", Importance::High)];
    let json = serde_json::to_value(run(&input)).unwrap();
    assert!(json["setup"].is_null());
    assert_eq!(json["stand_aside"], true);
    assert!(
        json["stand_aside_reason"]
            .as_str()
            .unwrap()
            .contains("FOMC")
    );
}

#[test]
fn rounding_follows_digits() {
    let view = run(&uptrend_input());
    let decimals = |v: f64, d: i32| {
        let f = 10f64.powi(d);
        (v * f - (v * f).round()).abs() < 1e-6
    };
    let setup = view.setup.unwrap();
    for p in [
        setup.entry_low,
        setup.entry_high,
        setup.stop_loss,
        setup.take_profit,
        setup.take_profit_2,
    ] {
        assert!(decimals(p, 2), "{p}");
    }
    assert!(decimals(setup.risk_reward, 2));
    assert!(decimals(setup.atr_h1, 3) && decimals(setup.atr_m15, 3));
    for row in &view.timeframes {
        assert!(decimals(row.ema20, 3) && decimals(row.ema50, 3) && decimals(row.atr, 3));
        assert!(decimals(row.rsi, 2));
    }
    assert!(decimals(view.levels.pivot.unwrap(), 3));
    assert!(decimals(view.levels.vwap.unwrap(), 3));
}

#[test]
fn summary_wording_for_a_setup() {
    let view = run(&uptrend_input());
    let s = view.setup.as_ref().unwrap();
    let expected_tail = format!(
        "Buy zone {:.2} to {:.2}, stop {:.2}, target {:.2}.",
        s.entry_low, s.entry_high, s.stop_loss, s.take_profit
    );
    assert!(view.summary.starts_with(&format!(
        "Bullish intraday bias, confidence {} of 100: ",
        view.confidence
    )));
    assert!(view.summary.ends_with(&expected_tail), "{}", view.summary);
    for word in ["will", "guarantee", "certain"] {
        assert!(!view.summary.contains(word), "{}", view.summary);
    }
}

#[test]
fn config_reads_partial_overrides() {
    let default = cfg();
    let parsed: ViewConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed, default);

    let parsed: ViewConfig =
        serde_json::from_str(r#"{"weight_news": 5.0, "horizon_minutes": 60, "unknown": 1}"#)
            .unwrap();
    assert_eq!(parsed.weight_news, 5.0);
    assert_eq!(parsed.horizon_minutes, 60);
    assert_eq!(parsed.weight_h4_trend, 25.0);

    let round: ViewConfig =
        serde_json::from_str(&serde_json::to_string(&default).unwrap()).unwrap();
    assert_eq!(round, default);
}

#[test]
fn config_defaults_match_the_specification() {
    let c = cfg();
    assert_eq!(
        [
            c.weight_h4_trend,
            c.weight_h1_trend,
            c.weight_m15_momentum,
            c.weight_m15_rsi,
            c.weight_vwap,
            c.weight_pivot,
            c.weight_news
        ],
        [25.0, 20.0, 15.0, 10.0, 10.0, 10.0, 10.0]
    );
    assert_eq!(c.bias_threshold, 20.0);
    assert_eq!(c.confidence_cap_on_disagreement, 50.0);
    assert_eq!(c.event_blackout_minutes, 30);
    assert_eq!(c.max_spread_atr_fraction, 0.25);
    assert_eq!(
        [
            c.entry_zone_atr_m15,
            c.stop_atr_h1,
            c.tp1_atr_h1,
            c.tp2_search_atr_h1,
            c.tp2_fallback_atr_h1
        ],
        [0.5, 1.0, 1.5, 3.0, 2.5]
    );
    assert_eq!(c.horizon_minutes, 240);
}

#[test]
fn input_types_serialise_as_documented() {
    assert_eq!(serde_json::to_string(&Timeframe::M15).unwrap(), "\"M15\"");
    assert_eq!(
        serde_json::from_str::<Timeframe>("\"H4\"").unwrap(),
        Timeframe::H4
    );
    assert!(Timeframe::M5 < Timeframe::M15 && Timeframe::H4 < Timeframe::D1);
    assert_eq!(
        serde_json::to_string(&Importance::Moderate).unwrap(),
        "\"moderate\""
    );
    assert!(Importance::Low < Importance::Moderate && Importance::Moderate < Importance::High);

    let input = uptrend_input();
    let json = serde_json::to_string(&input).unwrap();
    let back: ViewInput = serde_json::from_str(&json).unwrap();
    // Floats may differ in the last digit after a text round trip; the structure may not.
    assert_eq!(back.symbol, input.symbol);
    assert_eq!(
        back.frames.keys().collect::<Vec<_>>(),
        input.frames.keys().collect::<Vec<_>>()
    );
    for (tf, bars) in &input.frames {
        let times: Vec<_> = bars.iter().map(|b| b.time).collect();
        assert_eq!(
            back.frames[tf].iter().map(|b| b.time).collect::<Vec<_>>(),
            times
        );
    }
}
