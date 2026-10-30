//! `get_rates` and `get_calendar`: the Rust client against the in-process mock EA.
#![cfg(feature = "mock")]

use std::time::{Duration, Instant};

use jarvis_bridge_client::mock::{MockBridge, MockState};
use jarvis_bridge_client::protocol::{
    CALENDAR_RANGE_MAX_SECS, CalendarQuery, Importance, RATES_COUNT_DEFAULT, RatesQuery, Timeframe,
};
use jarvis_bridge_client::{BridgeClient, BridgeError, RATES_RETRY_DELAY};

const TIMEOUT: Duration = Duration::from_millis(500);
/// A fixed mock clock (13:20:30 server time) so two runs give identical data.
const CLOCK: i64 = 1_793_366_430;

const SYMBOLS: [&str; 6] = ["XAUUSD", "US100", "US30", "EURUSD", "GBPUSD", "BTCUSD"];

async fn started() -> (MockBridge, BridgeClient) {
    let state = MockState {
        clock: Some(CLOCK),
        ..MockState::default()
    };
    let mock = MockBridge::start_with(state).await;
    let client = BridgeClient::new(mock.config(TIMEOUT)).expect("client");
    (mock, client)
}

fn rates_query(symbol: &str, timeframe: Timeframe, count: Option<u32>) -> RatesQuery {
    RatesQuery {
        symbol: symbol.to_owned(),
        timeframe,
        count,
    }
}

fn day(from: i64) -> CalendarQuery {
    CalendarQuery {
        from,
        to: from + 48 * 3600,
        currencies: Vec::new(),
        min_importance: None,
    }
}

#[tokio::test]
async fn bars_are_well_formed_for_every_symbol_and_timeframe() {
    let (mock, client) = started().await;
    for symbol in SYMBOLS {
        let digits = mock
            .state()
            .prices
            .iter()
            .find(|p| p.symbol == symbol)
            .unwrap()
            .digits;
        for timeframe in Timeframe::ALL {
            let rates = client
                .get_rates(rates_query(symbol, timeframe, Some(120)))
                .await
                .unwrap_or_else(|e| panic!("{symbol} {timeframe}: {e}"));
            assert_eq!(
                (rates.symbol.as_str(), rates.timeframe, rates.digits),
                (symbol, timeframe, digits)
            );
            assert_eq!(rates.bars.len(), 120);
            let step = timeframe.seconds();
            let scale = 10f64.powi(i32::try_from(digits).unwrap());
            for bar in &rates.bars {
                let ctx = format!("{symbol} {timeframe} {bar:?}");
                assert_eq!(bar.time % step, 0, "{ctx}: not aligned to the timeframe");
                assert!(bar.high >= bar.open.max(bar.close), "{ctx}");
                assert!(bar.low <= bar.open.min(bar.close), "{ctx}");
                assert!(
                    bar.low > 0.0 && bar.tick_volume > 0 && bar.spread >= 0,
                    "{ctx}"
                );
                for price in [bar.open, bar.high, bar.low, bar.close] {
                    assert!(
                        ((price * scale).round() - price * scale).abs() < 1e-6,
                        "{ctx}: digits"
                    );
                }
            }
            // Oldest first, consecutive bars, the last one is the bar the clock is in.
            for pair in rates.bars.windows(2) {
                assert_eq!(pair[1].time - pair[0].time, step);
                assert_eq!(
                    pair[1].open, pair[0].close,
                    "{symbol} {timeframe}: bars are contiguous"
                );
            }
            assert_eq!(rates.bars.last().unwrap().time, CLOCK - CLOCK % step);
        }
    }
}

#[tokio::test]
async fn last_close_sits_at_the_mock_price() {
    let (mock, client) = started().await;
    for symbol in SYMBOLS {
        let bid = mock
            .state()
            .prices
            .iter()
            .find(|p| p.symbol == symbol)
            .unwrap()
            .bid;
        for timeframe in [Timeframe::M1, Timeframe::M15, Timeframe::D1] {
            let rates = client
                .get_rates(rates_query(symbol, timeframe, None))
                .await
                .unwrap();
            assert_eq!(
                rates.bars.last().unwrap().close,
                bid,
                "{symbol} {timeframe}"
            );
        }
    }
    // The series follows a price that moves.
    mock.update_state(|s| s.prices[0].bid = 2400.5);
    let rates = client
        .get_rates(rates_query("XAUUSD", Timeframe::M5, Some(10)))
        .await
        .unwrap();
    assert_eq!(rates.bars.last().unwrap().close, 2400.5);
}

#[tokio::test]
async fn default_count_is_300_and_the_maximum_is_served() {
    let (_mock, client) = started().await;
    let default = client
        .get_rates(rates_query("EURUSD", Timeframe::H1, None))
        .await
        .unwrap();
    assert_eq!(default.bars.len(), RATES_COUNT_DEFAULT as usize);
    let max = client
        .get_rates(rates_query("EURUSD", Timeframe::H1, Some(1000)))
        .await
        .unwrap();
    assert_eq!(max.bars.len(), 1000);
    // A shorter request is the tail of a longer one.
    assert_eq!(max.bars[700..], default.bars[..]);
}

#[tokio::test]
async fn bars_are_deterministic_across_two_runs_and_differ_between_series() {
    let (_mock_a, client_a) = started().await;
    let (_mock_b, client_b) = started().await;
    let a = client_a
        .get_rates(rates_query("XAUUSD", Timeframe::M15, Some(200)))
        .await
        .unwrap();
    let b = client_b
        .get_rates(rates_query("XAUUSD", Timeframe::M15, Some(200)))
        .await
        .unwrap();
    assert_eq!(a, b);
    let other_timeframe = client_a
        .get_rates(rates_query("XAUUSD", Timeframe::M5, Some(200)))
        .await
        .unwrap();
    assert_ne!(a.bars[..50], other_timeframe.bars[..50]);
    let other_symbol = client_a
        .get_rates(rates_query("US100", Timeframe::M15, Some(200)))
        .await
        .unwrap();
    assert_ne!(a.bars[0].open, other_symbol.bars[0].open);
}

#[tokio::test]
async fn volatility_is_realistic_per_instrument() {
    let (_mock, client) = started().await;
    let mean_range = |symbol: &'static str| {
        let client = client.clone();
        async move {
            let rates = client
                .get_rates(rates_query(symbol, Timeframe::M15, Some(1000)))
                .await
                .unwrap();
            let sum: f64 = rates.bars.iter().map(|b| b.high - b.low).sum();
            sum / rates.bars.len() as f64
        }
    };
    let gold = mean_range("XAUUSD").await;
    assert!((2.0..15.0).contains(&gold), "gold M15 range {gold} dollars");
    let eurusd_pips = mean_range("EURUSD").await * 10_000.0;
    assert!(
        (2.0..15.0).contains(&eurusd_pips),
        "EURUSD M15 range {eurusd_pips} pips"
    );
}

#[tokio::test]
async fn unknown_symbol_and_bad_count_are_errors() {
    let (mock, client) = started().await;
    match client
        .get_rates(rates_query("NOPE", Timeframe::M15, None))
        .await
    {
        Err(BridgeError::Remote { retcode: 4301, .. }) => {}
        other => panic!("expected 4301, got {other:?}"),
    }
    // Rejected on the client side: nothing reaches the EA.
    for count in [0, 1001, u32::MAX] {
        match client
            .get_rates(rates_query("XAUUSD", Timeframe::M15, Some(count)))
            .await
        {
            Err(BridgeError::Remote {
                retcode: -1,
                message,
            }) => {
                assert!(message.contains("between 1 and 1000"), "{message}");
            }
            other => panic!("count {count}: expected -1, got {other:?}"),
        }
    }
    assert_eq!(
        mock.request_count(),
        1,
        "only the unknown-symbol request was sent"
    );
}

#[tokio::test]
async fn history_not_ready_is_retried_once_after_500_ms() {
    let (mock, client) = started().await;
    mock.history_not_ready(1);
    let started_at = Instant::now();
    let rates = client
        .get_rates(rates_query("XAUUSD", Timeframe::M15, Some(5)))
        .await
        .expect("the retry succeeds");
    assert_eq!(rates.bars.len(), 5);
    assert_eq!(mock.request_count(), 2, "one failed request and one retry");
    assert!(
        started_at.elapsed() >= RATES_RETRY_DELAY,
        "the retry waited {:?}",
        started_at.elapsed()
    );
    assert_eq!(RATES_RETRY_DELAY, Duration::from_millis(500));
}

#[tokio::test]
async fn history_not_ready_twice_is_returned_after_a_single_retry() {
    let (mock, client) = started().await;
    mock.history_not_ready(5);
    match client
        .get_rates(rates_query("XAUUSD", Timeframe::M15, Some(5)))
        .await
    {
        Err(BridgeError::Remote { retcode: 4401, .. }) => {}
        other => panic!("expected 4401, got {other:?}"),
    }
    assert_eq!(mock.request_count(), 2, "never more than one retry");
}

#[tokio::test]
async fn other_errors_are_not_retried() {
    let (mock, client) = started().await;
    assert!(
        client
            .get_rates(rates_query("NOPE", Timeframe::M1, None))
            .await
            .is_err()
    );
    assert_eq!(mock.request_count(), 1);
}

#[tokio::test]
async fn calendar_has_mixed_events_over_the_next_48_hours() {
    let (_mock, client) = started().await;
    let query = CalendarQuery {
        from: CLOCK - 86_400,
        to: CLOCK + 48 * 3600,
        currencies: Vec::new(),
        min_importance: Some(Importance::Low),
    };
    let calendar = client.get_calendar(query).await.unwrap();
    let events = &calendar.events;
    assert!(events.len() >= 8, "{} events", events.len());
    assert!(
        events.windows(2).all(|w| w[0].time <= w[1].time),
        "sorted by time"
    );
    for currency in ["USD", "EUR", "GBP", "JPY"] {
        assert!(events.iter().any(|e| e.currency == currency), "{currency}");
    }
    for importance in [Importance::Low, Importance::Moderate, Importance::High] {
        assert!(
            events.iter().any(|e| e.importance == importance),
            "{importance}"
        );
    }
    // Released events have an actual; some upcoming ones have none, and some have no forecast.
    assert!(events.iter().any(|e| e.time <= CLOCK && e.actual.is_some()));
    assert!(events.iter().any(|e| e.time > CLOCK && e.actual.is_none()));
    assert!(
        events
            .iter()
            .all(|e| e.time > CLOCK || e.forecast.is_none() || e.actual.is_some())
    );
    assert!(
        events
            .iter()
            .any(|e| e.forecast.is_none() && e.previous.is_none())
    );
    assert!(
        events
            .iter()
            .all(|e| !e.country.is_empty() && !e.event.is_empty())
    );
    let ahead = events
        .iter()
        .filter(|e| e.time > CLOCK && e.time <= CLOCK + 48 * 3600)
        .count();
    assert!(ahead >= 6, "{ahead} upcoming events");
}

#[tokio::test]
async fn calendar_filters_by_currency() {
    let (_mock, client) = started().await;
    let all = client.get_calendar(day(CLOCK)).await.unwrap().events;
    let usd = client
        .get_calendar(CalendarQuery {
            currencies: vec!["USD".into()],
            ..day(CLOCK)
        })
        .await
        .unwrap()
        .events;
    assert!(!usd.is_empty() && usd.len() < all.len());
    assert!(usd.iter().all(|e| e.currency == "USD"));
    assert_eq!(
        usd.len(),
        all.iter().filter(|e| e.currency == "USD").count()
    );

    let pair = client
        .get_calendar(CalendarQuery {
            currencies: vec!["eur".into(), "GBP".into()],
            ..day(CLOCK)
        })
        .await
        .unwrap()
        .events;
    assert!(
        pair.iter()
            .all(|e| e.currency == "EUR" || e.currency == "GBP")
    );
    assert!(pair.iter().any(|e| e.currency == "EUR") && pair.iter().any(|e| e.currency == "GBP"));
}

#[tokio::test]
async fn calendar_filters_by_importance_and_defaults_to_moderate() {
    let (_mock, client) = started().await;
    let at_least = |importance: Option<Importance>| {
        let client = client.clone();
        async move {
            client
                .get_calendar(CalendarQuery {
                    min_importance: importance,
                    ..day(CLOCK - 86_400)
                })
                .await
                .unwrap()
                .events
        }
    };
    let low = at_least(Some(Importance::Low)).await;
    let default = at_least(None).await;
    let moderate = at_least(Some(Importance::Moderate)).await;
    let high = at_least(Some(Importance::High)).await;
    assert_eq!(default, moderate, "the default is moderate");
    assert!(low.len() > moderate.len() && moderate.len() > high.len() && !high.is_empty());
    assert!(low.iter().any(|e| e.importance == Importance::Low));
    assert!(
        moderate
            .iter()
            .all(|e| e.importance >= Importance::Moderate)
    );
    assert!(high.iter().all(|e| e.importance == Importance::High));
}

#[tokio::test]
async fn calendar_only_returns_events_inside_the_range() {
    let (_mock, client) = started().await;
    let all = client
        .get_calendar(day(CLOCK - 86_400))
        .await
        .unwrap()
        .events;
    let some = &all[all.len() / 2];
    let narrow = client
        .get_calendar(CalendarQuery {
            from: some.time,
            to: some.time + 1,
            currencies: Vec::new(),
            min_importance: None,
        })
        .await
        .unwrap()
        .events;
    assert!(!narrow.is_empty());
    assert!(narrow.iter().all(|e| e.time == some.time));
}

#[tokio::test]
async fn calendar_bad_ranges_are_refused_before_sending() {
    let (mock, client) = started().await;
    for (from, to) in [
        (CLOCK, CLOCK),
        (CLOCK, CLOCK - 1),
        (CLOCK, CLOCK + CALENDAR_RANGE_MAX_SECS + 1),
    ] {
        match client
            .get_calendar(CalendarQuery {
                from,
                to,
                currencies: Vec::new(),
                min_importance: None,
            })
            .await
        {
            Err(BridgeError::Remote { retcode: -1, .. }) => {}
            other => panic!("{from}..{to}: expected -1, got {other:?}"),
        }
    }
    match client
        .get_calendar(CalendarQuery {
            currencies: vec!["DOLLAR".into()],
            ..day(CLOCK)
        })
        .await
    {
        Err(BridgeError::Remote {
            retcode: -1,
            message,
        }) => assert!(message.contains("DOLLAR")),
        other => panic!("expected -1, got {other:?}"),
    }
    assert_eq!(mock.request_count(), 0, "nothing reached the EA");
    // The longest range is accepted.
    let longest = CalendarQuery {
        from: CLOCK,
        to: CLOCK + CALENDAR_RANGE_MAX_SECS,
        currencies: Vec::new(),
        min_importance: None,
    };
    assert!(client.get_calendar(longest).await.is_ok());
}

#[tokio::test]
async fn the_commands_are_known_to_the_mock() {
    let (mock, client) = started().await;
    client.get_calendar(day(CLOCK)).await.unwrap();
    client
        .get_rates(rates_query("EURUSD", Timeframe::M1, Some(1)))
        .await
        .unwrap();
    assert_eq!(mock.request_count(), 2);
}
