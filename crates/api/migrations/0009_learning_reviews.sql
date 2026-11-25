-- Learning reviews (spec-architecture-page): what was sent to Claude (statistics only), what
-- came back, and the checked proposals. Items record the config version they were applied as.
CREATE TABLE learning_reviews (
    id            TEXT PRIMARY KEY,
    created_at    TEXT NOT NULL,
    created_ms    INTEGER NOT NULL,
    provider      TEXT NOT NULL,
    model         TEXT NOT NULL,
    live_version  INTEGER,
    input_json    TEXT NOT NULL,
    output_text   TEXT NOT NULL,
    summary       TEXT,
    items_json    TEXT NOT NULL,
    rejected_json TEXT NOT NULL,
    cost_usd      REAL NOT NULL DEFAULT 0
);

CREATE INDEX learning_reviews_created ON learning_reviews (created_ms);
