//! Append-only audit log in SQLite (SPEC rule 10).
//!
//! Only [`Audit::record`] writes. A failed write is logged loudly but never fails the
//! chat turn that triggered it.

use std::str::FromStr;

use anyhow::Context;
use chrono::{SecondsFormat, Utc};
use jarvis_gate::KillSwitchState;
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// What a row records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    Utterance,
    ToolCall,
    BridgeRequest,
    BridgeReply,
    AssistantReply,
    Error,
    /// What the deterministic parser understood from the utterance.
    Intent,
    /// A preview was issued (token issued, never stored in full).
    Preview,
    /// The gate, the intent cross-check or the kill switch refused a proposal.
    GateRejected,
    Confirm,
    ConfirmSecond,
    Cancel,
    /// Outcome of one bridge write.
    TradeResult,
    KillSwitch,
    Login,
    LoginFailed,
    VoiceStt,
    VoiceTts,
    /// The tighten-only trading limits were set or reset from the UI.
    LimitsChanged,
    /// The model's final reply stated figures that no tool, user message or clock reported.
    /// The model got one retry.
    UngroundedReply,
    /// The retried reply was ungrounded again and was replaced with a safe reply.
    UngroundedReplyBlocked,
    /// A broker's symbols were read and mapped (start-up, account switch or on request).
    SymbolMapDetected,
    /// The user set or cleared a symbol mapping in Settings.
    SymbolMapOverride,
    /// A market view was computed (symbol, bias, confidence, stand-aside).
    MarketView,
    /// A market view was turned into a trade preview request from the UI.
    MarketPrepare,
    /// One Claude request in Assistant mode: tokens, cost and the month's total.
    ClaudeUsage,
    /// "Remember that ...": a note was written to the captures folder.
    NoteCaptured,
    /// A 3D model was built (or changed) in Assistant mode.
    ModelBuilt,
    /// A Desk specialist reported a finding (spec-nimbus-desk).
    DeskFinding,
    /// Sentinel approved or vetoed a Desk trade plan.
    SentinelVerdict,
    /// Pilot turned an approved plan into a trade preview (or was refused).
    PilotPreview,
    /// The trader saved a review of a decision (spec-architecture-page).
    DecisionReviewed,
    /// Council weights or Desk guards were applied or rolled back.
    CouncilConfigChanged,
    /// A learning review was asked of Claude (statistics sent, proposals received).
    LearningReview,
    /// The Desk master account lock was set or changed.
    AccountLockChanged,
    /// The user's preferences (persona, provider, voice, watchlist, aliases) were changed.
    SettingsChanged,
    /// A request was refused before it reached a handler: a foreign `Origin`, a non-loopback
    /// `Host` while login is off, or a used-up request budget (AUD-1). At most one row per
    /// reason and caller per minute.
    RequestRefused,
}

impl AuditKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditKind::Utterance => "utterance",
            AuditKind::ToolCall => "tool_call",
            AuditKind::BridgeRequest => "bridge_request",
            AuditKind::BridgeReply => "bridge_reply",
            AuditKind::AssistantReply => "assistant_reply",
            AuditKind::Error => "error",
            AuditKind::Intent => "intent",
            AuditKind::Preview => "preview",
            AuditKind::GateRejected => "gate_rejected",
            AuditKind::Confirm => "confirm",
            AuditKind::ConfirmSecond => "confirm_second",
            AuditKind::Cancel => "cancel",
            AuditKind::TradeResult => "trade_result",
            AuditKind::KillSwitch => "kill_switch",
            AuditKind::Login => "login",
            AuditKind::LoginFailed => "login_failed",
            AuditKind::VoiceStt => "voice_stt",
            AuditKind::VoiceTts => "voice_tts",
            AuditKind::LimitsChanged => "limits_changed",
            AuditKind::UngroundedReply => "ungrounded_reply",
            AuditKind::UngroundedReplyBlocked => "ungrounded_reply_blocked",
            AuditKind::SymbolMapDetected => "symbol_map_detected",
            AuditKind::SymbolMapOverride => "symbol_map_override",
            AuditKind::MarketView => "market_view",
            AuditKind::MarketPrepare => "market_prepare",
            AuditKind::ClaudeUsage => "claude_usage",
            AuditKind::NoteCaptured => "note_captured",
            AuditKind::ModelBuilt => "model_built",
            AuditKind::DeskFinding => "desk_finding",
            AuditKind::SentinelVerdict => "sentinel_verdict",
            AuditKind::PilotPreview => "pilot_preview",
            AuditKind::DecisionReviewed => "decision_reviewed",
            AuditKind::CouncilConfigChanged => "council_config_changed",
            AuditKind::LearningReview => "learning_review",
            AuditKind::AccountLockChanged => "account_lock_changed",
            AuditKind::SettingsChanged => "settings_changed",
            AuditKind::RequestRefused => "request_refused",
        }
    }
}

tokio::task_local! {
    /// The peer address of the request being handled on this task.
    static CLIENT_IP: String;
}

fn current_client_ip() -> Option<String> {
    CLIENT_IP.try_with(Clone::clone).ok()
}

/// Middleware: rows audited while this request is handled carry its peer address (AUD-1).
/// Work a handler spawns onto another task is not covered; it records no `ip`. A request
/// without a peer address (an in-process caller) gets none either.
pub async fn with_client_ip(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip().to_string());
    match peer {
        Some(ip) => CLIENT_IP.scope(ip, next.run(req)).await,
        None => next.run(req).await,
    }
}

/// One stored row, as read back.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub id: i64,
    pub ts: String,
    pub session_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    /// The persona that was active for this step; `None` for old rows and for steps that
    /// belong to no conversation.
    pub persona: Option<String>,
}

/// Handle to the audit table. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Audit {
    pool: SqlitePool,
}

impl Audit {
    /// Open (creating the file and its directory if needed), enable WAL, run migrations.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::from_str(url)
            .with_context(|| format!("invalid database.url '{url}'"))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let path = options.get_filename();
        if path != std::path::Path::new(":memory:")
            && let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
        {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating database directory {}", dir.display()))?;
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .with_context(|| format!("opening database '{url}'"))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("running audit migrations")?;
        Ok(Self { pool })
    }

    /// Append one row that belongs to no persona. Failures are logged at error level and
    /// swallowed.
    pub async fn record(&self, session_id: Option<Uuid>, kind: AuditKind, payload: Value) {
        self.record_as(session_id, None, kind, payload).await;
    }

    /// Append one row, noting which persona was active (rule 10). A row written while a request
    /// is being handled gets the client's address as `ip` (see [`with_client_ip`]) unless the
    /// payload already names one.
    pub async fn record_as(
        &self,
        session_id: Option<Uuid>,
        persona: Option<&str>,
        kind: AuditKind,
        mut payload: Value,
    ) {
        if let (Some(ip), Some(map)) = (current_client_ip(), payload.as_object_mut()) {
            map.entry("ip").or_insert_with(|| Value::from(ip));
        }
        let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let result = sqlx::query(
            "INSERT INTO audit (ts, session_id, kind, payload, persona)              VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(ts)
        .bind(session_id.map(|s| s.to_string()))
        .bind(kind.as_str())
        .bind(payload.to_string())
        .bind(persona)
        .execute(&self.pool)
        .await;
        if let Err(e) = result {
            tracing::error!(
                kind = kind.as_str(),
                error = %e,
                "AUDIT WRITE FAILED: this step is missing from the audit log"
            );
        }
    }

    /// Read rows in insertion order, optionally for one session. Used by tests and the
    /// future audit viewer.
    pub async fn rows(&self, session_id: Option<Uuid>) -> anyhow::Result<Vec<AuditRow>> {
        let rows = match session_id {
            Some(id) => {
                sqlx::query(
                    "SELECT id, ts, session_id, kind, payload, persona FROM audit \
                     WHERE session_id = ?1 ORDER BY id",
                )
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT id, ts, session_id, kind, payload, persona FROM audit ORDER BY id",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.into_iter().map(row_from).collect()
    }

    /// Newest first, at most `limit` rows with `id < before_id` (all rows when `None`).
    /// Backs `GET /api/audit`.
    pub async fn recent(
        &self,
        limit: u32,
        before_id: Option<i64>,
    ) -> anyhow::Result<Vec<AuditRow>> {
        let rows = sqlx::query(
            "SELECT id, ts, session_id, kind, payload, persona FROM audit \
             WHERE (?1 IS NULL OR id < ?1) ORDER BY id DESC LIMIT ?2",
        )
        .bind(before_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_from).collect()
    }

    /// The newest rows written for one persona (Desk mode's journal is its `desk` rows).
    pub async fn recent_for_persona(
        &self,
        persona: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<AuditRow>> {
        let rows = sqlx::query(
            "SELECT id, ts, session_id, kind, payload, persona FROM audit \
             WHERE persona = ?1 ORDER BY id DESC LIMIT ?2",
        )
        .bind(persona)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_from).collect()
    }

    /// Remember the kill switch so it survives a restart. One row, replaced on every change.
    pub async fn save_kill_switch(&self, state: &KillSwitchState) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO kill_switch (id, engaged, reason, changed_at_ms) VALUES (1, ?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET engaged = excluded.engaged, reason = excluded.reason, changed_at_ms = excluded.changed_at_ms",
        )
        .bind(state.engaged)
        .bind(state.reason.as_deref())
        .bind(state.changed_at_ms)
        .execute(&self.pool)
        .await
        .context("saving the kill switch state")?;
        Ok(())
    }

    /// The last saved kill switch state; `None` if it was never changed.
    pub async fn load_kill_switch(&self) -> anyhow::Result<Option<KillSwitchState>> {
        let row =
            sqlx::query("SELECT engaged, reason, changed_at_ms FROM kill_switch WHERE id = 1")
                .fetch_optional(&self.pool)
                .await
                .context("reading the kill switch state")?;
        row.map(|r| {
            Ok(KillSwitchState {
                engaged: r.try_get::<bool, _>("engaged")?,
                reason: r.try_get("reason")?,
                changed_at_ms: r.try_get("changed_at_ms")?,
            })
        })
        .transpose()
    }

    /// The shared SQLite pool (the auth tables live in the same database).
    #[doc(hidden)]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the pool (tests that then reopen the file).
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

fn row_from(r: sqlx::sqlite::SqliteRow) -> anyhow::Result<AuditRow> {
    let payload: String = r.try_get("payload")?;
    Ok(AuditRow {
        id: r.try_get("id")?,
        ts: r.try_get("ts")?,
        session_id: r.try_get("session_id")?,
        kind: r.try_get("kind")?,
        payload: serde_json::from_str(&payload)?,
        persona: r.try_get("persona")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_db_url(name: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("jarvis-audit-{name}-{}", Uuid::new_v4()));
        let url = format!(
            "sqlite://{}",
            dir.join("nested")
                .join("audit.db")
                .to_string_lossy()
                .replace('\\', "/")
        );
        (dir, url)
    }

    #[tokio::test]
    async fn creates_directory_and_appends_rows() {
        let (dir, url) = temp_db_url("append");
        let audit = Audit::connect(&url).await.unwrap();
        let sid = Uuid::new_v4();
        audit
            .record(Some(sid), AuditKind::Utterance, json!({"text": "hi"}))
            .await;
        audit
            .record(None, AuditKind::Error, json!({"message": "boom"}))
            .await;
        let rows = audit.rows(None).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "utterance");
        assert_eq!(
            rows[0].session_id.as_deref(),
            Some(sid.to_string().as_str())
        );
        assert_eq!(rows[0].payload["text"], "hi");
        assert_eq!(rows[1].session_id, None);
        assert_eq!(audit.rows(Some(sid)).await.unwrap().len(), 1);
        audit.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn rows_carry_the_persona_when_one_was_active() {
        let (dir, url) = temp_db_url("persona");
        let audit = Audit::connect(&url).await.unwrap();
        audit.record(None, AuditKind::KillSwitch, json!({})).await;
        audit
            .record_as(None, Some("friday"), AuditKind::Utterance, json!({}))
            .await;
        // A row written before the column existed reads back as None.
        sqlx::query(
            "INSERT INTO audit (ts, session_id, kind, payload) VALUES ('t', NULL, 'old', '{}')",
        )
        .execute(&audit.pool)
        .await
        .unwrap();
        let rows = audit.rows(None).await.unwrap();
        let personas: Vec<_> = rows.iter().map(|r| r.persona.as_deref()).collect();
        assert_eq!(personas, [None, Some("friday"), None]);
        let recent = audit.recent(10, None).await.unwrap();
        assert_eq!(recent[1].persona.as_deref(), Some("friday"));
        // Still append-only with the new column.
        assert!(
            sqlx::query("UPDATE audit SET persona = 'x'")
                .execute(&audit.pool)
                .await
                .is_err()
        );
        audit.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn recent_pages_newest_first() {
        let (dir, url) = temp_db_url("recent");
        let audit = Audit::connect(&url).await.unwrap();
        for i in 0..5 {
            audit
                .record(None, AuditKind::Utterance, json!({ "n": i }))
                .await;
        }
        let first = audit.recent(2, None).await.unwrap();
        assert_eq!(
            first
                .iter()
                .map(|r| r.payload["n"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [4, 3]
        );
        let next = audit.recent(10, Some(first[1].id)).await.unwrap();
        assert_eq!(
            next.iter()
                .map(|r| r.payload["n"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [2, 1, 0]
        );
        audit.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn table_is_append_only() {
        let (dir, url) = temp_db_url("appendonly");
        let audit = Audit::connect(&url).await.unwrap();
        audit.record(None, AuditKind::Utterance, json!({})).await;
        let update = sqlx::query("UPDATE audit SET kind = 'x'")
            .execute(&audit.pool)
            .await;
        assert!(update.is_err());
        let delete = sqlx::query("DELETE FROM audit").execute(&audit.pool).await;
        assert!(delete.is_err());
        assert_eq!(audit.rows(None).await.unwrap().len(), 1);
        audit.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn write_failure_does_not_panic() {
        let (dir, url) = temp_db_url("closed");
        let audit = Audit::connect(&url).await.unwrap();
        audit.close().await;
        // Pool closed: the insert fails, record() must log and return.
        audit.record(None, AuditKind::Error, json!({})).await;
        let _ = std::fs::remove_dir_all(dir);
    }
}
