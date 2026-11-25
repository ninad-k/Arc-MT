-- The Desk master account lock (ADR-0025): the MT5 account number the master bot may trade on,
-- as the trader set it in the UI. One row; none means the `[desk] master_account` default.
CREATE TABLE account_lock (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    expected_login INTEGER NOT NULL,
    broker         TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    actor          TEXT
);
