-- The market view journal. Every view that is computed is stored here and later scored
-- against what the price did (outcome, bias_correct, move_atr). Unlike the audit table this
-- one is updated by the evaluator.
--
-- Times: created_at and valid_until are UTC (ISO 8601) for display; created_server and
-- valid_server are the same instants on the broker's server clock (unix seconds), which is
-- the clock of the price bars the evaluator compares them with.
CREATE TABLE market_views (
    id             TEXT PRIMARY KEY,
    created_at     TEXT NOT NULL,
    created_server INTEGER NOT NULL,
    symbol         TEXT NOT NULL,
    broker_symbol  TEXT NOT NULL,
    bias           TEXT NOT NULL,
    confidence     INTEGER NOT NULL,
    score          REAL NOT NULL,
    stand_aside    INTEGER NOT NULL,
    -- The setup, when there is one: side, middle of the entry zone, stop and first target.
    side           TEXT,
    entry          REAL,
    stop           REAL,
    tp             REAL,
    -- Middle of bid and ask when the view was made, and the H1 ATR used to size moves.
    price          REAL NOT NULL,
    atr_h1         REAL,
    valid_until    TEXT NOT NULL,
    valid_server   INTEGER NOT NULL,
    view_json      TEXT NOT NULL,
    -- open | tp_hit | sl_hit | expired | no_setup
    outcome        TEXT NOT NULL DEFAULT 'open',
    evaluated_at   TEXT,
    bias_correct   INTEGER,
    move_atr       REAL,
    -- 1 once the bias has been judged (or can no longer be) after valid_until.
    bias_done      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX market_views_created_idx ON market_views (created_at DESC);
CREATE INDEX market_views_symbol_idx ON market_views (symbol, created_at DESC);
CREATE INDEX market_views_pending_idx ON market_views (outcome, bias_done);
