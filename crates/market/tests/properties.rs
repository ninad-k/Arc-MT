//! Property-style checks over many seeded random walks: `analyze` never panics, stays
//! inside its documented ranges and keeps every setup in a sensible order.

mod common;

use std::collections::BTreeMap;

use common::*;
use jarvis_market::{
    Bar, Bias, EventInput, Importance, MarketView, NewsInput, Side, Timeframe, ViewConfig,
    ViewInput, analyze,
};

const SLOTS: usize = 15_000; // five-minute steps, about 52 days of history

fn random_input(seed: u64) -> ViewInput {
    let mut rng = Rng(seed.wrapping_mul(0x1234_5678_9ABC_DEF1) ^ 0xDEAD_BEEF);
    let digits = [0u32, 2, 3, 5][(rng.next_u64() % 4) as usize];
    let base = 1.0 + rng.unit() * 5000.0;
    let vol = base * (0.0001 + rng.unit() * 0.002);
    let drift = rng.signed() * vol * 0.15;

    let mut walk = Vec::with_capacity(SLOTS);
    let mut p = base;
    for _ in 0..SLOTS {
        p = (p + drift + rng.signed() * vol).max(base * 0.05);
        walk.push(p);
    }
    let start = NOW - (SLOTS as i64 - 1) * 300;
    let path = move |t: i64| walk[(((t - start) / 300).clamp(0, SLOTS as i64 - 1)) as usize];

    let mut frames: BTreeMap<Timeframe, Vec<Bar>> = BTreeMap::new();
    for (tf, n) in [
        (Timeframe::M5, 300),
        (Timeframe::M15, 300),
        (Timeframe::H1, 300),
        (Timeframe::H4, 300),
        (Timeframe::D1, 30),
    ] {
        // Some cases get short or missing frames.
        let n = match rng.next_u64() % 6 {
            0 => (rng.next_u64() % 60) as usize,
            1 => 0,
            _ => n,
        };
        let mut bars = frame(tf, n, NOW, vol * 0.3, &path);
        // Occasionally corrupt a bar.
        if !bars.is_empty() && rng.unit() < 0.1 {
            let i = (rng.next_u64() as usize) % bars.len();
            match rng.next_u64() % 3 {
                0 => bars[i].high = f64::NAN,
                1 => bars[i].close = f64::INFINITY,
                _ => bars[i].low = bars[i].high + 1.0,
            }
        }
        if rng.unit() < 0.08 {
            bars.reverse();
        }
        frames.insert(tf, bars);
    }

    let mid = path(NOW);
    let spread = match rng.next_u64() % 8 {
        0 => mid * 0.02,
        1 => 0.0,
        _ => vol * 0.05 * rng.unit(),
    };
    let (bid, ask) = match rng.next_u64() % 25 {
        0 => (mid + 1.0, mid), // crossed
        1 => (f64::NAN, mid),
        _ => (mid, mid + spread),
    };

    let currencies = vec!["USD".to_string()];
    let events = (0..(rng.next_u64() % 4))
        .map(|_| EventInput {
            time: NOW + ((rng.signed() * 36.0 * 3600.0) as i64),
            currency: ["USD", "EUR", "JPY"][(rng.next_u64() % 3) as usize].into(),
            title: format!("event {}", rng.next_u64() % 100),
            importance: [Importance::Low, Importance::Moderate, Importance::High]
                [(rng.next_u64() % 3) as usize],
        })
        .collect();
    let news = match rng.next_u64() % 4 {
        0 => Some(NewsInput {
            score: rng.signed() * 1.5,
            count: (rng.next_u64() % 20) as u32,
        }),
        1 => Some(NewsInput {
            score: f64::NAN,
            count: 1,
        }),
        _ => None,
    };

    ViewInput {
        symbol: "TEST".into(),
        digits,
        point: 10f64.powi(-(digits as i32)),
        bid,
        ask,
        now: NOW,
        frames,
        currencies,
        events,
        news,
    }
}

fn finite(v: f64, what: &str) {
    assert!(v.is_finite(), "{what} is not finite");
}

fn check_view(view: &MarketView, digits: u32) {
    assert!(view.confidence <= 100);
    assert!(
        (-100.0..=100.0).contains(&view.score),
        "score {}",
        view.score
    );
    finite(view.score, "score");
    finite(view.price.bid, "bid");
    finite(view.price.ask, "ask");
    for f in &view.factors {
        finite(f.points, "factor points");
        assert!(f.points.abs() <= 100.0);
    }
    if !view.factors.is_empty() {
        let sum: f64 = view.factors.iter().map(|f| f.points).sum();
        assert!((view.score - sum.clamp(-100.0, 100.0)).abs() < 1e-6);
    }
    for r in &view.timeframes {
        for v in [r.ema20, r.ema50, r.rsi, r.macd_hist, r.atr] {
            finite(v, "timeframe value");
        }
        assert!((0.0..=100.0).contains(&r.rsi));
    }
    let l = &view.levels;
    for v in [
        l.pivot,
        l.r1,
        l.r2,
        l.s1,
        l.s2,
        l.prev_day_high,
        l.prev_day_low,
        l.vwap,
    ]
    .into_iter()
    .flatten()
    .chain(l.support.iter().copied())
    .chain(l.resistance.iter().copied())
    {
        finite(v, "level");
    }
    finite(view.news.score, "news score");
    assert_eq!(view.stand_aside, view.stand_aside_reason.is_some());
    if view.stand_aside || view.bias == Bias::Neutral {
        assert!(view.setup.is_none());
    }
    if view.factors.is_empty() {
        assert_eq!((view.bias, view.confidence), (Bias::Neutral, 0));
    }
    assert!(!view.summary.is_empty());
    assert!(
        serde_json::to_string(view).is_ok(),
        "serialisation must not fail"
    );

    if let Some(s) = &view.setup {
        for v in [
            s.entry_low,
            s.entry_high,
            s.stop_loss,
            s.take_profit,
            s.take_profit_2,
            s.risk_reward,
            s.atr_h1,
            s.atr_m15,
        ] {
            finite(v, "setup value");
        }
        assert!(s.risk_reward > 0.0);
        assert!(s.atr_h1 > 0.0);
        match s.side {
            Side::Buy => {
                assert_eq!(view.bias, Bias::Bullish);
                assert!(s.stop_loss < s.entry_low, "{s:?}");
                assert!(s.entry_low <= s.entry_high, "{s:?}");
                assert!(s.entry_high < s.take_profit, "{s:?}");
                assert!(s.take_profit <= s.take_profit_2, "{s:?}");
            }
            Side::Sell => {
                assert_eq!(view.bias, Bias::Bearish);
                assert!(s.stop_loss > s.entry_high, "{s:?}");
                assert!(s.entry_low <= s.entry_high, "{s:?}");
                assert!(s.entry_low > s.take_profit, "{s:?}");
                assert!(s.take_profit >= s.take_profit_2, "{s:?}");
            }
        }
        // Prices carry at most `digits` decimals.
        let f = 10f64.powi(digits as i32);
        for p in [
            s.entry_low,
            s.entry_high,
            s.stop_loss,
            s.take_profit,
            s.take_profit_2,
        ] {
            assert!(
                (p * f - (p * f).round()).abs() < 1e-6 * f.max(1.0),
                "{p} digits {digits}"
            );
        }
    }
}

#[test]
fn random_walks_never_break_the_invariants() {
    let cfg = ViewConfig::default();
    let (mut buys, mut sells, mut aside, mut thin) = (0, 0, 0, 0);
    for seed in 0..400u64 {
        let input = random_input(seed);
        let view = analyze(&input, &cfg);
        check_view(&view, input.digits);
        assert_eq!(
            view,
            analyze(&input, &cfg),
            "seed {seed}: not deterministic"
        );
        match view.setup.as_ref().map(|s| s.side) {
            Some(Side::Buy) => buys += 1,
            Some(Side::Sell) => sells += 1,
            None => {}
        }
        aside += usize::from(view.stand_aside);
        thin += usize::from(view.factors.is_empty());
    }
    // Make sure the generator really exercises the interesting paths.
    assert!(buys >= 10 && sells >= 10, "buys {buys}, sells {sells}");
    assert!(aside >= 5, "stand-aside cases: {aside}");
    assert!(thin >= 5, "thin-data cases: {thin}");
}

#[test]
fn random_configs_never_panic() {
    let mut rng = Rng(99);
    for seed in 0..100u64 {
        let input = random_input(seed + 1000);
        let cfg = ViewConfig {
            weight_h4_trend: rng.signed() * 60.0,
            weight_news: rng.signed() * 60.0,
            bias_threshold: rng.unit() * 100.0,
            event_blackout_minutes: (rng.unit() * 600.0) as i64 - 100,
            max_spread_atr_fraction: rng.signed(),
            entry_zone_atr_m15: rng.signed() * 2.0,
            stop_atr_h1: rng.signed() * 3.0,
            tp1_atr_h1: rng.signed() * 3.0,
            tp2_fallback_atr_h1: rng.signed() * 3.0,
            horizon_minutes: i64::MAX / 2,
            swing_left: (rng.next_u64() % 6) as usize,
            swing_right: (rng.next_u64() % 6) as usize,
            levels_per_side: (rng.next_u64() % 5) as usize,
            min_factors: (rng.next_u64() % 9) as usize,
            ..ViewConfig::default()
        };
        let view = analyze(&input, &cfg);
        assert!(view.confidence <= 100);
        assert!(serde_json::to_string(&view).is_ok());
    }
}
