use jarvis_news::{FinnhubData, NewsError};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "test-key-123";

fn client(server: &MockServer) -> FinnhubData {
    FinnhubData::new(KEY)
        .unwrap()
        .with_base_url(&server.uri())
        .unwrap()
}

#[tokio::test]
async fn insider_transactions_parse_the_documented_shape() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/stock/insider-transactions"))
        .and(query_param("symbol", "TSLA"))
        .and(query_param("from", "2021-01-01"))
        .and(header("x-finnhub-token", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                { "name": "Kirkhorn Zachary", "share": 57234, "change": -1250, "filingDate": "2021-03-19",
                  "transactionDate": "2021-03-17", "transactionCode": "S", "transactionPrice": 655.81 },
                { "name": "Baglino Andrew D", "share": 20614, "change": 1000, "filingDate": "2021-03-31",
                  "transactionDate": "2021-03-29", "transactionCode": "M", "transactionPrice": 41.57 }
            ],
            "symbol": "TSLA"
        })))
        .mount(&server)
        .await;
    let rows = client(&server)
        .insider_transactions("TSLA", "2021-01-01", "2021-03-31")
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].change, Some(-1250.0));
    assert_eq!(rows[0].transaction_code.as_deref(), Some("S"));
    assert_eq!(rows[1].transaction_price, Some(41.57));
}

#[tokio::test]
async fn basic_financials_read_known_keys() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/stock/metric"))
        .and(query_param("metric", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metric": { "52WeekHigh": 310.43, "52WeekLow": 149.22, "beta": 1.2989,
                        "peBasicExclExtraTTM": 24.5, "52WeekLowDate": "2019-01-14" },
            "metricType": "all", "series": {}, "symbol": "AAPL"
        })))
        .mount(&server)
        .await;
    let f = client(&server).basic_financials("AAPL").await.unwrap();
    assert_eq!(f.week52_high(), Some(310.43));
    assert_eq!(f.week52_low(), Some(149.22));
    assert_eq!(f.beta(), Some(1.2989));
    assert_eq!(f.pe_ttm(), Some(24.5));
    assert_eq!(f.eps_ttm(), None);
}

#[tokio::test]
async fn filings_parse_a_bare_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/stock/filings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "accessNumber": "0001193125-20-050884", "symbol": "AAPL", "cik": "320193", "form": "8-K",
              "filedDate": "2020-02-27 00:00:00", "acceptedDate": "2020-02-27 06:14:21",
              "reportUrl": "https://www.sec.gov/r", "filingUrl": "https://www.sec.gov/f" }
        ])))
        .mount(&server)
        .await;
    let rows = client(&server)
        .filings("AAPL", "2020-01-01", "2020-06-11")
        .await
        .unwrap();
    assert_eq!(rows[0].form, "8-K");
    assert_eq!(rows[0].filed_date.as_deref(), Some("2020-02-27 00:00:00"));
}

#[tokio::test]
async fn a_premium_endpoint_on_a_free_key_is_not_configured() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/stock/congressional-trading"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string(format!("You don't have access to this resource. {KEY}")),
        )
        .mount(&server)
        .await;
    let err = client(&server)
        .congressional_trades("AAPL", "2026-01-01", "2026-11-24")
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::NotConfigured), "{err}");
}

#[tokio::test]
async fn congressional_trades_parse_when_allowed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/stock/congressional-trading"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "amountFrom": 1001, "amountTo": 15000, "assetName": "X", "filingDate": "2015-05-14",
                       "name": "Lamar Alexander", "ownerType": "Spouse", "position": "senator",
                       "symbol": "MLPTX", "transactionDate": "2014-02-07", "transactionType": "Purchase" }],
            "symbol": "MLPTX"
        })))
        .mount(&server)
        .await;
    let rows = client(&server)
        .congressional_trades("MLPTX", "2014-01-01", "2015-01-01")
        .await
        .unwrap();
    assert_eq!(rows[0].transaction_type.as_deref(), Some("Purchase"));
    assert_eq!(rows[0].amount_to, Some(15000.0));
}

#[tokio::test]
async fn errors_never_carry_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string(format!("boom {KEY}")))
        .mount(&server)
        .await;
    let err = client(&server).basic_financials("AAPL").await.unwrap_err();
    assert!(!err.to_string().contains(KEY), "{err}");
    assert!(FinnhubData::new("  ").is_err());
    assert!(
        FinnhubData::new(KEY)
            .unwrap()
            .with_base_url("http://example.com")
            .is_err()
    );
}
