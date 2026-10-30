//! Synthetic market data for the mock EA: price bars and an economic calendar.
//!
//! Bars are a seeded random walk, built backwards from the mock's current bid so that the
//! last close always sits at the quote. The noise of bar `k` depends only on the seed (a hash
//! of symbol and timeframe) and on `k`, the absolute bar number since the epoch. That makes
//! the series deterministic for a given clock, and a short request is always the tail of a
//! longer one.
use crate::mock::MockState;
use crate::mock::{err_reply, ok_reply, unix_now, unknown_symbol};
use crate::protocol::{
    Bar, CALENDAR_EVENTS_MAX, Calendar, CalendarEvent, CalendarQuery, Importance, Price,
    RATES_COUNT_DEFAULT, RATES_COUNT_MAX, RETCODE_INVALID_REQUEST, Rates, RatesQuery, Reply,
    Timeframe,
};

/// MT5 `ERR_HISTORY_NOT_FOUND`: what the EA answers while a symbol's history is still loading.
const RETCODE_HISTORY_NOT_FOUND: i64 = 4401;

/// `get_rates`.
pub(crate) fn rates(state: &mut MockState, id: &str, query: &RatesQuery) -> Reply {
    let count = query.count.unwrap_or(RATES_COUNT_DEFAULT);
    if !(1..=RATES_COUNT_MAX).contains(&count) {
        return err_reply(
            id,
            RETCODE_INVALID_REQUEST,
            &format!("'count' must be between 1 and {RATES_COUNT_MAX}"),
        );
    }
    let Some(price) = state
        .prices
        .iter()
        .find(|p| p.symbol == query.symbol)
        .cloned()
    else {
        return unknown_symbol(id, &query.symbol);
    };
    if state.history_not_ready > 0 {
        state.history_not_ready -= 1;
        return err_reply(
            id,
            RETCODE_HISTORY_NOT_FOUND,
            &format!(
                "history of {} {} is not loaded yet",
                query.symbol, query.timeframe
            ),
        );
    }
    let now = state.clock.unwrap_or_else(unix_now);
    ok_reply(id, &synthesize(&price, query.timeframe, count, now))
}

/// `get_calendar`.
pub(crate) fn calendar(state: &MockState, id: &str, query: &CalendarQuery) -> Reply {
    if let Err(message) = query.validate() {
        return err_reply(id, RETCODE_INVALID_REQUEST, &message);
    }
    let now = state.clock.unwrap_or_else(unix_now);
    let min = query.min_importance.unwrap_or(Importance::Moderate);
    let mut events: Vec<CalendarEvent> = canned_calendar(now)
        .into_iter()
        .filter(|e| e.time >= query.from && e.time <= query.to)
        .filter(|e| e.importance >= min)
        .filter(|e| {
            query.currencies.is_empty()
                || query
                    .currencies
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&e.currency))
        })
        .collect();
    events.sort_by(|a, b| a.time.cmp(&b.time).then_with(|| a.event.cmp(&b.event)));
    events.truncate(CALENDAR_EVENTS_MAX);
    ok_reply(id, &Calendar { events })
}

// ---- bars ---------------------------------------------------------------------------------

/// Typical close-to-close move of one M15 bar as a fraction of the price.
fn m15_move(symbol: &str) -> f64 {
    match symbol {
        "XAUUSD" => 0.0013, // about 3 dollars at 2400
        "US100" => 0.0011,
        "US30" => 0.0008,
        "EURUSD" => 0.00035, // about 4 pips
        "GBPUSD" => 0.00042,
        "BTCUSD" => 0.0024,
        _ => 0.0007,
    }
}

fn synthesize(price: &Price, timeframe: Timeframe, count: u32, now: i64) -> Rates {
    let secs = timeframe.seconds();
    let digits = price.digits;
    let scale = 10f64.powi(i32::try_from(digits).unwrap_or(0));
    let round = |x: f64| (x * scale).round() / scale;
    let seed = fnv1a(price.symbol.as_bytes(), timeframe.as_str().as_bytes());
    // Wider timeframes move more, with the square root of the bar length.
    let move_fraction = m15_move(&price.symbol) * (secs as f64 / 900.0).sqrt();
    let count = count as usize;

    // Absolute number of the forming bar and of the first bar returned.
    let last_k = now.div_euclid(secs);
    let first_k = last_k - (count as i64 - 1);

    // closes[i] is the open of bar i and closes[i + 1] its close; the last one is the quote.
    let mut closes = vec![0.0_f64; count + 1];
    closes[count] = price.bid;
    for i in (0..count).rev() {
        let k = first_k + i as i64;
        let step = (move_fraction * gauss(seed, k, 1)).exp();
        closes[i] = closes[i + 1] / step;
    }

    let spread_base = price.spread_points.max(0);
    let bars = (0..count)
        .map(|i| {
            let k = first_k + i as i64;
            let open = round(closes[i]);
            let close = round(closes[i + 1]);
            let wick = move_fraction * open * 0.5;
            let high = round(open.max(close) + wick * gauss(seed, k, 2).abs());
            let low = round((open.min(close) - wick * gauss(seed, k, 3).abs()).max(1.0 / scale));
            let activity = 200.0 + (mix(seed, k, 4) % 800) as f64;
            let tick_volume = (activity * (secs as f64 / 60.0).sqrt()).max(1.0) as u64;
            let jitter = i64::try_from(mix(seed, k, 5) % (spread_base as u64 / 4 + 1)).unwrap_or(0);
            Bar {
                time: k * secs,
                open,
                high,
                low,
                close,
                tick_volume,
                spread: spread_base + jitter,
            }
        })
        .collect();
    Rates {
        symbol: price.symbol.clone(),
        timeframe,
        digits,
        bars,
    }
}

/// FNV-1a over both byte strings: the seed of one (symbol, timeframe) series.
fn fnv1a(a: &[u8], b: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in a.iter().chain(std::iter::once(&b'/')).chain(b) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// splitmix64 finaliser: a well-mixed 64-bit value from the seed, bar number and a salt that
/// tells the uses of the noise apart.
fn mix(seed: u64, k: i64, salt: u64) -> u64 {
    let mut z = seed
        .wrapping_add((k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .wrapping_add(salt.wrapping_mul(0xbf58_476d_1ce4_e5b9));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A standard normal value (Box-Muller), clamped to +-4 so one bar cannot explode.
fn gauss(seed: u64, k: i64, salt: u64) -> f64 {
    let unit = |x: u64| ((x >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    let u1 = unit(mix(seed, k, salt * 2 + 100));
    let u2 = unit(mix(seed, k, salt * 2 + 101));
    ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()).clamp(-4.0, 4.0)
}

// ---- calendar -----------------------------------------------------------------------------

/// One row of the canned calendar: minutes from the top of the current hour, currency,
/// event, importance, unit, forecast, previous.
type Row = (
    i64,
    &'static str,
    &'static str,
    Importance,
    &'static str,
    Option<f64>,
    Option<f64>,
);

const CANNED: [Row; 16] = [
    (
        -1260,
        "USD",
        "Core PCE Price Index m/m",
        Importance::High,
        "%",
        Some(0.3),
        Some(0.2),
    ),
    (
        -180,
        "GBP",
        "Retail Sales m/m",
        Importance::Moderate,
        "%",
        Some(0.4),
        Some(-0.3),
    ),
    (
        30,
        "EUR",
        "German Ifo Business Climate",
        Importance::Moderate,
        "",
        Some(87.5),
        Some(87.0),
    ),
    (
        90,
        "USD",
        "ADP Employment Change",
        Importance::Moderate,
        "K",
        Some(150.0),
        Some(143.0),
    ),
    (
        150,
        "USD",
        "ISM Services PMI",
        Importance::Moderate,
        "",
        Some(51.8),
        Some(51.5),
    ),
    (
        300,
        "EUR",
        "ECB Interest Rate Decision",
        Importance::High,
        "%",
        Some(3.25),
        Some(3.5),
    ),
    (
        330,
        "USD",
        "Initial Jobless Claims",
        Importance::Low,
        "K",
        Some(225.0),
        Some(222.0),
    ),
    (
        540,
        "USD",
        "FOMC Member Speaks",
        Importance::Low,
        "",
        None,
        None,
    ),
    (
        720,
        "JPY",
        "BoJ Summary of Opinions",
        Importance::Moderate,
        "",
        None,
        None,
    ),
    (
        1230,
        "JPY",
        "Tokyo Core CPI y/y",
        Importance::Moderate,
        "%",
        Some(2.1),
        Some(2.0),
    ),
    (
        1500,
        "GBP",
        "GDP m/m",
        Importance::High,
        "%",
        Some(0.2),
        Some(0.1),
    ),
    (
        1710,
        "USD",
        "Nonfarm Payrolls",
        Importance::High,
        "K",
        Some(185.0),
        Some(175.0),
    ),
    (
        1710,
        "USD",
        "Unemployment Rate",
        Importance::High,
        "%",
        Some(4.1),
        Some(4.1),
    ),
    (
        2100,
        "EUR",
        "CPI Flash Estimate y/y",
        Importance::High,
        "%",
        Some(2.0),
        Some(2.1),
    ),
    (
        2400,
        "GBP",
        "BoE Governor Speaks",
        Importance::Moderate,
        "",
        None,
        None,
    ),
    (
        2640,
        "USD",
        "Fed Chair Speaks",
        Importance::High,
        "",
        None,
        None,
    ),
];

fn country_of(currency: &str) -> &'static str {
    match currency {
        "USD" => "US",
        "EUR" => "EU",
        "GBP" => "GB",
        "JPY" => "JP",
        _ => "",
    }
}

/// The canned events around `now`: two already released (with an `actual`) and the rest spread
/// over the next 48 hours. Event times are anchored to the top of the current hour, so they
/// move with the clock instead of going stale in a long-running mock.
pub(crate) fn canned_calendar(now: i64) -> Vec<CalendarEvent> {
    let base = now - now.rem_euclid(3600);
    CANNED
        .iter()
        .map(
            |&(minutes, currency, event, importance, unit, forecast, previous)| {
                let time = base + minutes * 60;
                // Released events get a value close to the forecast; the rest have none yet.
                let actual = if time <= now {
                    forecast.map(|f| ((f + 0.1) * 100.0).round() / 100.0)
                } else {
                    None
                };
                CalendarEvent {
                    time,
                    currency: currency.to_owned(),
                    country: country_of(currency).to_owned(),
                    event: event.to_owned(),
                    importance,
                    actual,
                    forecast,
                    previous,
                    unit: unit.to_owned(),
                }
            },
        )
        .collect()
}
