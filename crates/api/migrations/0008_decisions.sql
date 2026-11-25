-- The architecture page (spec-architecture-page): one row per decision, from the signal to the
-- trade or the reason there was none. Unlike the audit table these rows are updated as the
-- decision moves on (master, preview, execution, later the outcome); the audit table stays the
-- record of every action.
CREATE TABLE decisions (
    id              TEXT PRIMARY KEY,
    created_at      TEXT NOT NULL,          -- ISO 8601 UTC
    created_ms      INTEGER NOT NULL,
    updated_ms      INTEGER NOT NULL,
    session_id      TEXT,
    symbol          TEXT NOT NULL,          -- canonical
    broker_symbol   TEXT,
    venue           TEXT NOT NULL,          -- mt5 | alpaca
    trigger_kind    TEXT NOT NULL,          -- scan | question | ask
    trigger_detail  TEXT,
    -- The furthest stage reached: signal | members | council | master | execution.
    stage           TEXT NOT NULL,
    -- open (still moving) | waiting (needs Ask Friday) | traded | stopped
    status          TEXT NOT NULL,
    stop_code       TEXT,
    stop_detail     TEXT,
    price           REAL,
    atr             REAL,
    score           REAL,
    threshold       REAL,
    council_passed  INTEGER NOT NULL DEFAULT 0,
    side            TEXT,                   -- bull | bear when the council passed
    council_json    TEXT,
    weights_version INTEGER,
    master_json     TEXT,                   -- model, system prompt hash, messages, reply
    plan_id         TEXT,
    plan_json       TEXT,
    verdict_json    TEXT,
    preview_id      TEXT,
    tickets_json    TEXT,
    -- Filled when the trade closes (result in R and money).
    result_r        REAL,
    pnl             REAL,
    closed_at       TEXT
);

CREATE INDEX decisions_created ON decisions (created_ms);
CREATE INDEX decisions_symbol ON decisions (symbol, created_ms);
CREATE INDEX decisions_plan ON decisions (plan_id);
CREATE INDEX decisions_preview ON decisions (preview_id);
CREATE INDEX decisions_status ON decisions (status, updated_ms);

-- Every member's vote on the decision, with what it saw. Resolved later against the price
-- (outcome hit | miss | flat), which the learning loop scores. Counterfactual = the decision
-- did not trade.
CREATE TABLE decision_opinions (
    decision_id    TEXT NOT NULL REFERENCES decisions (id),
    agent          TEXT NOT NULL,
    stance         TEXT NOT NULL,
    confidence     INTEGER NOT NULL,
    weight         REAL NOT NULL,
    contribution   REAL NOT NULL,
    horizon        TEXT NOT NULL,
    evidence_json  TEXT NOT NULL,
    sources_json   TEXT NOT NULL,
    data_json      TEXT,
    mapper         TEXT NOT NULL,
    outcome        TEXT,
    move_atr       REAL,
    resolved_ms    INTEGER,
    PRIMARY KEY (decision_id, agent)
);

CREATE INDEX decision_opinions_unresolved ON decision_opinions (resolved_ms, agent);
CREATE INDEX decision_opinions_agent ON decision_opinions (agent, decision_id);

-- The trader's own analysis of a decision. The one table the user edits; each save is audited.
CREATE TABLE decision_reviews (
    decision_id   TEXT PRIMARY KEY REFERENCES decisions (id),
    setup         TEXT,
    mistakes_json TEXT NOT NULL DEFAULT '[]',
    regime        TEXT,
    rating        INTEGER,
    take_again    INTEGER,
    notes         TEXT,
    updated_at    TEXT NOT NULL
);

-- Council weights and Desk guards, versioned. The newest row is live; rolling back copies an
-- older row as a new version, so history is never rewritten.
CREATE TABLE config_versions (
    version      INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at   TEXT NOT NULL,
    weights_json TEXT NOT NULL,
    guards_json  TEXT NOT NULL,
    reason       TEXT NOT NULL,
    proposal_id  TEXT,
    actor        TEXT
);
