//! Deterministic synthetic market data for the `analyze` tests.
#![allow(dead_code)]

use std::collections::BTreeMap;

use jarvis_market::{Bar, EventInput, Importance, Timeframe, ViewInput};

/// 2026-10-30 00:00 on the input clock.
pub const DAY0: i64 = 1_793_404_800;
/// 12:07 on that day: the M5 frame has 144 closed bars today.
pub const NOW: i64 = DAY0 + 12 * 3600 + 7 * 60;

/// Small deterministic generator (SplitMix64) so the tests need no extra crate.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [-1, 1).
    pub fn signed(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }
}

/// Bars for one frame sampled from a continuous price path. The last bar is the
/// forming one (it opens at the start of the period containing `now`).
pub fn frame(
    tf: Timeframe,
    count: usize,
    now: i64,
    vol: f64,
    path: &dyn Fn(i64) -> f64,
) -> Vec<Bar> {
    let period = tf.seconds();
    let last_open = now.div_euclid(period) * period;
    let wick = vol * (period as f64 / 300.0).sqrt();
    (0..count)
        .map(|i| {
            let time = last_open - (count - 1 - i) as i64 * period;
            let open = path(time);
            let close = path((time + period).min(now));
            Bar {
                time,
                open,
                high: open.max(close) + wick,
                low: open.min(close) - wick,
                close,
                tick_volume: 100 + (i as u64 * 37) % 50,
                spread: 20,
            }
        })
        .collect()
}

pub fn frames(now: i64, vol: f64, path: &dyn Fn(i64) -> f64) -> BTreeMap<Timeframe, Vec<Bar>> {
    let mut map = BTreeMap::new();
    for tf in [Timeframe::M5, Timeframe::M15, Timeframe::H1, Timeframe::H4] {
        map.insert(tf, frame(tf, 300, now, vol, path));
    }
    map.insert(Timeframe::D1, frame(Timeframe::D1, 30, now, vol, path));
    map
}

pub fn input_from(path: &dyn Fn(i64) -> f64, vol: f64, spread: f64) -> ViewInput {
    let bid = path(NOW);
    ViewInput {
        symbol: "XAUUSD".into(),
        digits: 2,
        point: 0.01,
        bid: (bid * 100.0).round() / 100.0,
        ask: ((bid + spread) * 100.0).round() / 100.0,
        now: NOW,
        frames: frames(NOW, vol, path),
        currencies: vec!["USD".into()],
        events: Vec::new(),
        news: None,
    }
}

/// Deterministic noise in [-1, 1) that depends only on the five-minute slot of `t`, so
/// every frame sees the same underlying path.
pub fn noise(t: i64) -> f64 {
    Rng(t.div_euclid(300) as u64 ^ 0xA5A5_5A5A).signed()
}

/// Rising 2.0 per hour with a little wobble and noise.
pub fn uptrend_path(t: i64) -> f64 {
    3900.0 + 2.0 * (t - NOW) as f64 / 3600.0 + 0.5 * (t as f64 / 1800.0).sin() + 1.5 * noise(t)
}

pub fn downtrend_path(t: i64) -> f64 {
    3900.0 - 2.0 * (t - NOW) as f64 / 3600.0 + 0.5 * (t as f64 / 1800.0).sin() + 1.5 * noise(t)
}

/// Sideways: a slow swing around 3900 that is small against the average range. The
/// phase is picked so the H4 and H1 trends come out mixed.
pub fn range_path(t: i64) -> f64 {
    3900.0 + 4.0 * (t as f64 / 20_000.0 + 20.3).sin() + 1.5 * noise(t)
}

/// An uptrend that spiked six hours ago and gave part of it back, so there is a swing
/// high above the price.
pub fn pullback_path(t: i64) -> f64 {
    let hours_from_peak = (t - (NOW - 6 * 3600)) as f64 / 3600.0;
    uptrend_path(t) + 30.0 * (-0.5 * hours_from_peak * hours_from_peak).exp()
}

pub fn uptrend_input() -> ViewInput {
    input_from(&uptrend_path, 1.0, 0.2)
}

pub fn downtrend_input() -> ViewInput {
    input_from(&downtrend_path, 1.0, 0.2)
}

pub fn range_input() -> ViewInput {
    input_from(&range_path, 1.0, 0.2)
}

pub fn pullback_input() -> ViewInput {
    input_from(&pullback_path, 1.0, 0.2)
}

pub fn event(
    minutes_from_now: i64,
    currency: &str,
    title: &str,
    importance: Importance,
) -> EventInput {
    EventInput {
        time: NOW + minutes_from_now * 60,
        currency: currency.into(),
        title: title.into(),
        importance,
    }
}
