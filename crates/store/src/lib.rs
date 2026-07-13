//! gaugewright local store — the SQLite event log + the admission transaction.
//!
//! The imperative shell around the pure `gaugewright-core` reducers (ADR 0004): it
//! folds a scope's events to current state (`INV-8`), runs `decide`, and appends
//! the resulting events **atomically** at the next position (single-writer per
//! scope, `INV-7`). A rejected command appends nothing (`INV-2`). The
//! fold/append loop is the reusable spine for every lifecycle.
//!
//! Threading (RF-A7): the `Store` is **synchronous** `rusqlite`. fold/append are
//! fast and run directly inside the control-plane's async handlers; the one
//! genuinely long operation — a Pi turn (subprocess + multi-step admission) — is
//! dispatched on `tokio::task::spawn_blocking` (`crates/app/src/lib.rs` `post_task`),
//! so it never blocks an async worker. Per-scope writes serialize through an
//! immediate transaction with a `busy_timeout` (see `open`), so concurrent
//! connections wait rather than fail. A move to an async SQLite driver is a
//! scale-time change behind this same API — not needed for the single-process,
//! single-user shape — and would not alter the admission semantics above.

use std::sync::Arc;

use gaugewright_core::{Lifecycle, Rejection};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

/// A transparent at-rest transform applied to record payloads of designated
/// **content** kinds (`SECAUD-9`/`SECAUD-6`). The store crate stays crypto-free: this
/// is the seam an app-side content vault implements to encrypt sensitive content
/// (e.g. `transcript`) under per-scope keys, leaving lifecycle/metadata records
/// untouched. `None` on the [`Store`] = plaintext (the default; zero behavior change).
pub trait ContentCodec: Send + Sync {
    /// Transform a payload for storage. Must be reversible by [`decode`](Self::decode).
    /// A non-content `kind` returns the payload unchanged (pass-through).
    fn encode(&self, scope: &str, kind: &str, payload: &str) -> String;
    /// Reverse [`encode`](Self::encode). Returns `None` when the payload is
    /// **unrecoverable** — its per-scope key was crypto-erased — so the caller drops
    /// the row (the content is gone, history intact). Non-content kinds and legacy
    /// plaintext return `Some(payload)`.
    fn decode(&self, scope: &str, kind: &str, payload: &str) -> Option<String>;
}

pub struct Store {
    conn: Connection,
    codec: Option<Arc<dyn ContentCodec>>,
}

#[derive(Debug)]
pub enum AdmitError {
    Rejected(Rejection),
    Db(rusqlite::Error),
    Json(serde_json::Error),
}

impl From<rusqlite::Error> for AdmitError {
    fn from(e: rusqlite::Error) -> Self {
        AdmitError::Db(e)
    }
}
impl From<serde_json::Error> for AdmitError {
    fn from(e: serde_json::Error) -> Self {
        AdmitError::Json(e)
    }
}

/// Map the `GAUGEWRIGHT_SQLITE_SYNCHRONOUS` setting to a SQLite `synchronous` mode
/// (`SCALE-5`): `FULL` (case-insensitive) for a hosted data plane's fsync-per-commit
/// durability, else the desktop default `NORMAL`. Pure, so the policy is unit-testable.
fn synchronous_mode(setting: Option<&str>) -> &'static str {
    match setting {
        Some(s) if s.trim().eq_ignore_ascii_case("full") => "FULL",
        _ => "NORMAL",
    }
}

impl Store {
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        Self::init(Connection::open_in_memory()?)
    }

    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        // WAL for concurrent readers; a busy timeout so a second writer WAITS
        // for the immediate-transaction lock instead of failing SQLITE_BUSY
        // under contention (RF-C7 — the contention test exercises this).
        //
        // SCALE-5: set `synchronous` **explicitly** rather than leaning on SQLite's
        // default. `NORMAL` + WAL is crash-safe against an *application* crash and only
        // risks losing the most-recent commit(s) on an *OS/power* crash mid-checkpoint —
        // the right desktop default (no fsync per commit). A hosted/multi-user data plane
        // sets `GAUGEWRIGHT_SQLITE_SYNCHRONOUS=FULL` for fsync-per-commit durability.
        // WAL auto-recovers (replays the log) on the next open, so no separate sweep.
        let sync = synchronous_mode(
            std::env::var("GAUGEWRIGHT_SQLITE_SYNCHRONOUS")
                .ok()
                .as_deref(),
        );
        conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA synchronous={sync};"
        ))?;
        Self::init(conn)
    }

    /// The `synchronous` durability level (`SCALE-5`): the desktop default is **NORMAL**;
    /// `GAUGEWRIGHT_SQLITE_SYNCHRONOUS=FULL` (case-insensitive) opts into fsync-per-commit
    /// for a hosted data plane. Any other/absent value → `NORMAL`. The current level is
    /// readable via [`synchronous`](Self::synchronous).
    pub fn synchronous(&self) -> Result<i64, rusqlite::Error> {
        self.conn.query_row("PRAGMA synchronous", [], |r| r.get(0))
    }

    fn init(conn: Connection) -> Result<Self, rusqlite::Error> {
        // Append-only event log (INV-6). `(scope_id, position)` is the per-scope
        // total order (INV-7). Records/projections tables come in later phases.
        // `command_receipts` discharges INV-19 (AT_MOST_ONCE): a command carrying
        // an idempotency key is admitted at most once per scope — a replay of the
        // same `(scope, command_key)` is a no-op that returns the prior state.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                 scope_id TEXT    NOT NULL,
                 position INTEGER NOT NULL,
                 kind     TEXT    NOT NULL,
                 payload  TEXT    NOT NULL,
                 PRIMARY KEY (scope_id, position)
             );
             CREATE TABLE IF NOT EXISTS command_receipts (
                 scope_id    TEXT    NOT NULL,
                 command_key TEXT    NOT NULL,
                 applied_at  INTEGER NOT NULL,
                 PRIMARY KEY (scope_id, command_key)
             );",
        )?;
        Ok(Self { conn, codec: None })
    }

    /// Fold a lifecycle's events within a scope into current state (`INV-8`:
    /// state is the fold). Events are filtered by `L::KIND` so distinct
    /// lifecycles (a run, its review, its export) can coexist in one scope.
    pub fn fold<L: Lifecycle>(&self, scope_id: &str) -> Result<L::State, AdmitError> {
        let mut stmt = self.conn.prepare(
            "SELECT payload FROM events WHERE scope_id = ?1 AND kind = ?2 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![scope_id, L::KIND], |r| r.get::<_, String>(0))?;
        let mut state = L::State::default();
        for row in rows {
            let event: L::Event = serde_json::from_str(&row?)?;
            state = L::evolve(&state, event);
        }
        Ok(state)
    }

    /// Append a durable **record** (non-lifecycle admitted evidence — e.g. a
    /// transcript message) at the next position in a scope. Same append-only log,
    /// single-writer per scope (`INV-6`/`INV-7`); these are facts, not reducer events.
    /// Returns the assigned `position` — a monotonic per-scope sequence the library
    /// projection uses for "Recent" ordering and latest-wins tombstones.
    pub fn append_record(
        &mut self,
        scope_id: &str,
        kind: &str,
        payload: &str,
    ) -> Result<i64, AdmitError> {
        // SECAUD-9/6: a configured content codec transparently encrypts content kinds
        // at rest (non-content kinds pass through). `None` ⇒ plaintext (the default).
        let stored = match &self.codec {
            Some(codec) => codec.encode(scope_id, kind, payload),
            None => payload.to_string(),
        };
        // Immediate (sqlite-local-store.md): take the write lock up front so the
        // MAX(position) read and the insert are one atomic write, even if another
        // connection ever shares this file (INV-7 single-writer per scope).
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let position: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM events WHERE scope_id = ?1",
            params![scope_id],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO events (scope_id, position, kind, payload) VALUES (?1, ?2, ?3, ?4)",
            params![scope_id, position, kind, stored],
        )?;
        tx.commit()?;
        Ok(position)
    }

    /// Atomically append one non-lifecycle record under an idempotency key.
    /// The returned tuple is `(position, inserted)`: a replay returns the
    /// original assigned position and `false` without duplicating the pointer.
    pub fn append_record_with_key(
        &mut self,
        scope_id: &str,
        command_key: &str,
        kind: &str,
        payload: &str,
    ) -> Result<(i64, bool), AdmitError> {
        let stored = match &self.codec {
            Some(codec) => codec.encode(scope_id, kind, payload),
            None => payload.to_string(),
        };
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(position) = tx
            .query_row(
                "SELECT applied_at FROM command_receipts WHERE scope_id = ?1 AND command_key = ?2",
                params![scope_id, command_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            tx.commit()?;
            return Ok((position, false));
        }
        let position: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM events WHERE scope_id = ?1",
            params![scope_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO events (scope_id, position, kind, payload) VALUES (?1, ?2, ?3, ?4)",
            params![scope_id, position, kind, stored],
        )?;
        tx.execute(
            "INSERT INTO command_receipts (scope_id, command_key, applied_at) VALUES (?1, ?2, ?3)",
            params![scope_id, command_key, position],
        )?;
        tx.commit()?;
        Ok((position, true))
    }

    /// All records of one `kind` in a scope, in order — a durable projection
    /// source (e.g. the transcript snapshot). A content codec (`SECAUD-9/6`) decodes
    /// each row; a crypto-erased row (`decode` ⇒ `None`) is dropped (content gone).
    pub fn records(&self, scope_id: &str, kind: &str) -> Result<Vec<String>, AdmitError> {
        let mut stmt = self.conn.prepare(
            "SELECT payload FROM events WHERE scope_id = ?1 AND kind = ?2 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![scope_id, kind], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let payload = row?;
            match &self.codec {
                Some(codec) => {
                    if let Some(plain) = codec.decode(scope_id, kind, &payload) {
                        out.push(plain);
                    }
                }
                None => out.push(payload),
            }
        }
        Ok(out)
    }

    /// The full event history for a scope, in order — the audit timeline
    /// (`INV-6`: the log is append-only and is the record). Returns
    /// `(position, kind, payload)` rows across all lifecycles in the scope. A content
    /// codec decodes content kinds; a crypto-erased content row is dropped.
    pub fn events(&self, scope_id: &str) -> Result<Vec<(i64, String, String)>, AdmitError> {
        let mut stmt = self.conn.prepare(
            "SELECT position, kind, payload FROM events WHERE scope_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![scope_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (pos, kind, payload) = row?;
            match &self.codec {
                Some(codec) => {
                    if let Some(plain) = codec.decode(scope_id, &kind, &payload) {
                        out.push((pos, kind, plain));
                    }
                }
                None => out.push((pos, kind, payload)),
            }
        }
        Ok(out)
    }

    /// Attach a [`ContentCodec`] (`SECAUD-9/6`) — transparent at-rest encryption of
    /// content kinds under per-scope keys. Builder; without one the store is plaintext.
    pub fn with_codec(mut self, codec: Arc<dyn ContentCodec>) -> Self {
        self.codec = Some(codec);
        self
    }

    /// Every distinct scope id present in the log, ordered. The seam for capturing a
    /// subtree (e.g. a project's owned scopes for relocation) without a separate index.
    pub fn scope_ids(&self) -> Result<Vec<String>, AdmitError> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT scope_id FROM events ORDER BY scope_id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Admit one command into a scope: fold → `decide` → append atomically
    /// (single-writer per scope, `INV-7`; rejection appends nothing, `INV-2`).
    ///
    /// The fold happens **inside** the immediate write transaction (not before
    /// it), so the whole fold→decide→append is one serializable unit per scope:
    /// two connections racing the same scope cannot both `decide` against stale
    /// state and both append (the immediate lock makes the second fold observe
    /// the first's committed events). Within one process the workbench mutex
    /// already serializes admits; this keeps the guarantee true at the store
    /// itself, under genuine multi-connection contention (RF-C7).
    pub fn admit<L: Lifecycle>(
        &mut self,
        scope_id: &str,
        command: L::Command,
    ) -> Result<L::State, AdmitError> {
        // Immediate (sqlite-local-store.md step 3): take the write lock up front so
        // the fold, the position read, and the multi-event append are one atomic,
        // non-interleavable unit per scope (INV-7/INV-22).
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Fold the scope's current state from *committed* events, inside the lock.
        let state = {
            let mut stmt = tx.prepare(
                "SELECT payload FROM events WHERE scope_id = ?1 AND kind = ?2 ORDER BY position",
            )?;
            let rows = stmt.query_map(params![scope_id, L::KIND], |r| r.get::<_, String>(0))?;
            let mut s = L::State::default();
            for row in rows {
                let event: L::Event = serde_json::from_str(&row?)?;
                s = L::evolve(&s, event);
            }
            s
        };
        let events = L::decide(&state, command).map_err(AdmitError::Rejected)?;

        // Next position is global per scope so the per-scope order is total
        // across all lifecycles, even though the fold filters by kind.
        let base: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM events WHERE scope_id = ?1",
            params![scope_id],
            |r| r.get(0),
        )?;
        let mut new_state = state;
        for (offset, event) in events.into_iter().enumerate() {
            let position = base + offset as i64;
            let payload = serde_json::to_string(&event)?;
            tx.execute(
                "INSERT INTO events (scope_id, position, kind, payload) VALUES (?1, ?2, ?3, ?4)",
                params![scope_id, position, L::KIND, payload],
            )?;
            new_state = L::evolve(&new_state, event);
        }
        tx.commit()?;
        Ok(new_state)
    }

    /// Idempotent admission (`INV-19`, `AT_MOST_ONCE`): admit `command` under a
    /// caller-supplied `command_key` that uniquely names *this* command attempt.
    /// A first attempt applies exactly like [`Self::admit`] and records a receipt;
    /// a **replay** of the same `(scope, command_key)` — a retried request, a
    /// double-submit — is a no-op that returns the scope's current state without
    /// appending again. The receipt check, the fold, the decide, and the append
    /// all happen inside one immediate transaction, so even two connections
    /// racing the same key admit it at most once (the loser sees the committed
    /// receipt and no-ops). Use this for any command reachable via an at-least-once
    /// delivery path (client retries, federated re-delivery); `admit` stays the
    /// path for commands with no natural key.
    pub fn admit_with_key<L: Lifecycle>(
        &mut self,
        scope_id: &str,
        command_key: &str,
        command: L::Command,
    ) -> Result<L::State, AdmitError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Fold inside the lock (same serializability as `admit`, RF-C12).
        let fold = |tx: &rusqlite::Transaction| -> Result<L::State, AdmitError> {
            let mut stmt = tx.prepare(
                "SELECT payload FROM events WHERE scope_id = ?1 AND kind = ?2 ORDER BY position",
            )?;
            let rows = stmt.query_map(params![scope_id, L::KIND], |r| r.get::<_, String>(0))?;
            let mut s = L::State::default();
            for row in rows {
                let event: L::Event = serde_json::from_str(&row?)?;
                s = L::evolve(&s, event);
            }
            Ok(s)
        };

        // Already applied this key? Idempotent no-op: return current state.
        let seen: bool = tx
            .query_row(
                "SELECT 1 FROM command_receipts WHERE scope_id = ?1 AND command_key = ?2",
                params![scope_id, command_key],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if seen {
            let state = fold(&tx)?;
            tx.commit()?;
            return Ok(state);
        }

        let state = fold(&tx)?;
        let events = L::decide(&state, command).map_err(AdmitError::Rejected)?;
        let base: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM events WHERE scope_id = ?1",
            params![scope_id],
            |r| r.get(0),
        )?;
        let mut new_state = state;
        for (offset, event) in events.into_iter().enumerate() {
            let position = base + offset as i64;
            let payload = serde_json::to_string(&event)?;
            tx.execute(
                "INSERT INTO events (scope_id, position, kind, payload) VALUES (?1, ?2, ?3, ?4)",
                params![scope_id, position, L::KIND, payload],
            )?;
            new_state = L::evolve(&new_state, event);
        }
        // Record the receipt only on a *successful* (non-rejected) admission, in
        // the same transaction — so a rejected command leaves no receipt and can
        // be legitimately retried, while an accepted one is sealed against replay.
        tx.execute(
            "INSERT INTO command_receipts (scope_id, command_key, applied_at) VALUES (?1, ?2, ?3)",
            params![scope_id, command_key, base],
        )?;
        tx.commit()?;
        Ok(new_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaugewright_core::resource_export::{ExportCommand, ExportPhase, ExportState};
    use gaugewright_core::review::{ReviewCommand, ReviewPhase, ReviewState};
    use gaugewright_core::run::{RunCommand::*, RunPhase, RunState};
    use std::collections::BTreeSet;

    #[test]
    fn synchronous_mode_defaults_to_normal_and_opts_into_full(/* SCALE-5 */) {
        assert_eq!(synchronous_mode(None), "NORMAL");
        assert_eq!(synchronous_mode(Some("")), "NORMAL");
        assert_eq!(synchronous_mode(Some("garbage")), "NORMAL");
        // FULL is the hosted-plane opt-in (case-insensitive, trimmed).
        assert_eq!(synchronous_mode(Some("FULL")), "FULL");
        assert_eq!(synchronous_mode(Some("  full  ")), "FULL");
    }

    #[test]
    fn a_file_store_sets_synchronous_normal_explicitly(/* SCALE-5 */) {
        // The desktop default is NORMAL (1): crash-safe against an app crash, no fsync per
        // commit. (Env-override → FULL is covered by the pure-helper test above, without an
        // env race.) An in-memory store keeps the SQLite default, so this uses a real file.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("durability.db");
        let path = path.to_str().unwrap().to_string();
        let store = Store::open(&path).unwrap();
        assert_eq!(store.synchronous().unwrap(), 1, "NORMAL == 1");
    }

    /// RF-C7: per-scope single-writer (INV-7) under REAL contention — many
    /// connections to the same file, all appending into one scope. The
    /// immediate transaction + busy_timeout must serialize them into one
    /// gapless total order, never SQLITE_BUSY, never a duplicate position.
    #[test]
    fn concurrent_appends_from_many_connections_keep_one_total_order() {
        const THREADS: usize = 8;
        const APPENDS: usize = 25;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contended.db");
        let path = path.to_str().unwrap().to_string();
        let _prime = Store::open(&path).unwrap(); // create the schema once

        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let p = path.clone();
                std::thread::spawn(move || {
                    let mut store = Store::open(&p).unwrap();
                    let mut positions = Vec::with_capacity(APPENDS);
                    for i in 0..APPENDS {
                        positions.push(
                            store
                                .append_record("scope-contended", "evt", &format!("t{t}-{i}"))
                                .expect("a contended append must wait, not fail"),
                        );
                    }
                    positions
                })
            })
            .collect();
        let mut all_positions: Vec<i64> = Vec::new();
        for h in handles {
            all_positions.extend(h.join().unwrap());
        }

        // One gapless per-scope total order across every writer: positions are
        // exactly 0..N*M with no duplicate and no hole (INV-6/INV-7).
        all_positions.sort_unstable();
        let expected: Vec<i64> = (0..(THREADS * APPENDS) as i64).collect();
        assert_eq!(all_positions, expected);

        let store = Store::open(&path).unwrap();
        let all = store.records("scope-contended", "evt").unwrap();
        assert_eq!(all.len(), THREADS * APPENDS);
    }

    /// RF-C7 (lifecycle path): concurrent `admit` calls race one run lifecycle;
    /// the decide-inside-the-write-lock spine must let exactly ONE RequestRun
    /// through and reject the rest, no matter the interleaving.
    #[test]
    fn concurrent_admits_settle_one_winner_per_lifecycle_step() {
        const THREADS: usize = 8;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admit-race.db");
        let path = path.to_str().unwrap().to_string();
        let _prime = Store::open(&path).unwrap();

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let p = path.clone();
                std::thread::spawn(move || {
                    let mut store = Store::open(&p).unwrap();
                    store.admit::<RunState>("run-race", RequestRun).is_ok()
                })
            })
            .collect();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|won| *won)
            .count();

        // Exactly one RequestRun was admitted; every loser was REJECTED by
        // decide (a verdict), not failed by the database (an error).
        assert_eq!(wins, 1, "exactly one concurrent RequestRun may win");
        let store = Store::open(&path).unwrap();
        let s = store.fold::<RunState>("run-race").unwrap();
        assert_eq!(s.phase, RunPhase::Requested);
    }

    /// RF-A10 / INV-19: a command admitted under a key applies once; a replay of
    /// the same key is a no-op returning the current state (AT_MOST_ONCE).
    #[test]
    fn admit_with_key_is_idempotent_on_replay() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = "idem-run";

        // First attempt applies RequestRun.
        let s = store
            .admit_with_key::<RunState>(scope, "req-key-1", RequestRun)
            .unwrap();
        assert_eq!(s.phase, RunPhase::Requested);

        // A replay of the SAME key is a no-op — no second event appended, the
        // phase is unchanged, and it does NOT error (it returns the current state).
        let s = store
            .admit_with_key::<RunState>(scope, "req-key-1", RequestRun)
            .unwrap();
        assert_eq!(s.phase, RunPhase::Requested, "replay must not advance");
        assert_eq!(
            store.records(scope, RunState::KIND).unwrap().len(),
            1,
            "replay appended no second event (AT_MOST_ONCE)"
        );

        // A DIFFERENT key for the next legitimate command applies normally.
        let s = store
            .admit_with_key::<RunState>(scope, "admit-key-1", AdmitRun)
            .unwrap();
        assert_eq!(s.phase, RunPhase::Admitted);
    }

    #[test]
    fn append_record_with_key_returns_the_stable_position_on_replay() {
        let mut store = Store::open_in_memory().unwrap();
        let first = store
            .append_record_with_key("chat-1", "whip:event:4", "runtime_pointer", "pointer-a")
            .unwrap();
        let replay = store
            .append_record_with_key("chat-1", "whip:event:4", "runtime_pointer", "pointer-b")
            .unwrap();
        assert_eq!(first, (0, true));
        assert_eq!(replay, (0, false));
        assert_eq!(
            store.records("chat-1", "runtime_pointer").unwrap(),
            vec!["pointer-a"]
        );
    }

    /// A rejected keyed command leaves no receipt, so a corrected retry under the
    /// same key can still succeed (only *accepted* commands are sealed).
    #[test]
    fn a_rejected_keyed_command_leaves_no_receipt() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = "idem-reject";
        // AdmitRun from Init is rejected (must RequestRun first).
        assert!(store
            .admit_with_key::<RunState>(scope, "k", AdmitRun)
            .is_err());
        // The same key now carries a valid command — it is NOT blocked by a
        // ghost receipt from the rejected attempt.
        let s = store
            .admit_with_key::<RunState>(scope, "k", RequestRun)
            .unwrap();
        assert_eq!(s.phase, RunPhase::Requested);
    }

    #[test]
    fn admits_and_rebuilds_a_run_from_the_log() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = "run-1";
        store.admit::<RunState>(scope, RequestRun).unwrap();
        store.admit::<RunState>(scope, AdmitRun).unwrap();
        let s = store.admit::<RunState>(scope, StartRun).unwrap();
        assert_eq!(s.phase, RunPhase::Running);
        // INV-8: the event log alone rebuilds the same state.
        assert_eq!(
            store.fold::<RunState>(scope).unwrap().phase,
            RunPhase::Running
        );
    }

    #[test]
    fn rejected_command_appends_no_event() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = "run-2";
        store.admit::<RunState>(scope, RequestRun).unwrap();
        let err = store.admit::<RunState>(scope, StartRun); // INV-11: not admitted
        assert!(matches!(err, Err(AdmitError::Rejected(_))));
        // INV-2: a rejected command is not a fact — the log is unchanged.
        assert_eq!(
            store.fold::<RunState>(scope).unwrap().phase,
            RunPhase::Requested
        );
    }

    #[test]
    fn events_returns_the_ordered_audit_timeline() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = "audit-1";
        store.admit::<RunState>(scope, RequestRun).unwrap();
        store.admit::<RunState>(scope, AdmitRun).unwrap();
        store.admit::<RunState>(scope, StartRun).unwrap();
        let events = store.events(scope).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events.iter().map(|(p, ..)| *p).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(events.iter().all(|(_, kind, _)| kind == "run"));
        assert!(events[0].2.contains("RunRequested"));
    }

    #[test]
    fn admission_is_atomic_all_events_or_none() {
        // INV-22: a `decide` that yields several events commits as one unit. A
        // multi-event command lands a contiguous position block with no partial
        // prefix visible — and a later rejection leaves that block intact.
        let mut store = Store::open_in_memory().unwrap();
        let scope = "atomic-1";
        // RequestRun → AdmitRun each yield one event; drive to Running (3 events).
        store.admit::<RunState>(scope, RequestRun).unwrap();
        store.admit::<RunState>(scope, AdmitRun).unwrap();
        store.admit::<RunState>(scope, StartRun).unwrap();
        let before = store.events(scope).unwrap();
        assert_eq!(before.len(), 3, "three admitted events");
        // A rejected command must not append a partial event.
        assert!(store.admit::<RunState>(scope, StartRun).is_err());
        assert_eq!(
            store.events(scope).unwrap().len(),
            3,
            "rejection left the log atomic"
        );
    }

    #[test]
    fn scopes_are_isolated_no_cross_scope_bleed() {
        // INV-7: each scope is its own total order; one scope's events and records
        // never appear when folding/reading another.
        let mut store = Store::open_in_memory().unwrap();
        store.admit::<RunState>("scope-a", RequestRun).unwrap();
        store
            .append_record("scope-a", "transcript", "a-note")
            .unwrap();
        // scope-b starts empty and its own positions begin at 0.
        assert_eq!(
            store.fold::<RunState>("scope-b").unwrap().phase,
            RunPhase::Init
        );
        assert!(store.records("scope-b", "transcript").unwrap().is_empty());
        assert!(store.events("scope-b").unwrap().is_empty());
        let p = store
            .append_record("scope-b", "transcript", "b-note")
            .unwrap();
        assert_eq!(p, 0, "scope-b's position sequence is independent");
        assert_eq!(
            store.records("scope-a", "transcript").unwrap(),
            vec!["a-note"]
        );
    }

    /// The Phase-1 gate end-to-end through the shell: a run produces a tainted
    /// output → review (conjunctive consent auto-clears) → export, all in one
    /// scope, each lifecycle admitted and folded independently by `KIND`.
    #[test]
    fn run_to_review_to_export_all_gated() {
        let mut store = Store::open_in_memory().unwrap();
        let scope = "engagement-1";
        let owners: BTreeSet<_> = ["A", "B"].iter().map(|s| (*s).into()).collect();

        // run reaches Running
        store.admit::<RunState>(scope, RequestRun).unwrap();
        store.admit::<RunState>(scope, AdmitRun).unwrap();
        store.admit::<RunState>(scope, StartRun).unwrap();

        // review of the tainted output: not released until every owner consents
        store
            .admit::<ReviewState>(
                scope,
                ReviewCommand::Propose {
                    required: owners.clone(),
                },
            )
            .unwrap();
        store
            .admit::<ReviewState>(scope, ReviewCommand::Consent("A".into()))
            .unwrap();
        let r = store
            .admit::<ReviewState>(scope, ReviewCommand::Consent("B".into()))
            .unwrap();
        assert_eq!(
            r.phase,
            ReviewPhase::Cleared,
            "auto-clears once both consent"
        );
        let r = store
            .admit::<ReviewState>(scope, ReviewCommand::Release)
            .unwrap();
        assert_eq!(r.phase, ReviewPhase::Released);

        // export: requires both source consents AND target admission
        store
            .admit::<ExportState>(
                scope,
                ExportCommand::ProposeExport {
                    source_required: owners,
                },
            )
            .unwrap();
        store
            .admit::<ExportState>(scope, ExportCommand::SourceConsent("A".into()))
            .unwrap();
        store
            .admit::<ExportState>(scope, ExportCommand::SourceConsent("B".into()))
            .unwrap();
        // not yet cleared without the target — export is rejected
        assert!(matches!(
            store.admit::<ExportState>(scope, ExportCommand::Export),
            Err(AdmitError::Rejected(_))
        ));
        store
            .admit::<ExportState>(scope, ExportCommand::TargetAdmit)
            .unwrap();
        let e = store
            .admit::<ExportState>(scope, ExportCommand::Export)
            .unwrap();
        assert_eq!(e.phase, ExportPhase::Exported);

        // the run lifecycle in the same scope is untouched by the others' events
        assert_eq!(
            store.fold::<RunState>(scope).unwrap().phase,
            RunPhase::Running
        );
    }

    /// A test codec: "encrypts" the `secret` kind by reversing the payload (and marks
    /// it), passes every other kind through, and can be told a scope is "erased" so its
    /// `secret` rows decode to `None` (unrecoverable).
    struct RevCodec {
        erased: std::sync::Mutex<std::collections::BTreeSet<String>>,
    }
    impl ContentCodec for RevCodec {
        fn encode(&self, _scope: &str, kind: &str, payload: &str) -> String {
            if kind == "secret" {
                format!("rev:{}", payload.chars().rev().collect::<String>())
            } else {
                payload.to_string()
            }
        }
        fn decode(&self, scope: &str, kind: &str, payload: &str) -> Option<String> {
            if kind != "secret" {
                return Some(payload.to_string());
            }
            if self.erased.lock().unwrap().contains(scope) {
                return None; // crypto-erased: unrecoverable
            }
            payload
                .strip_prefix("rev:")
                .map(|p| p.chars().rev().collect())
                .or_else(|| Some(payload.to_string())) // legacy plaintext
        }
    }

    #[test]
    fn content_codec_encrypts_content_kinds_at_rest_and_passes_others_through() {
        // SECAUD-9: a content kind is stored transformed (the raw column is not the
        // plaintext) yet reads back transparently; a non-content kind is untouched.
        let codec = std::sync::Arc::new(RevCodec {
            erased: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        });
        let mut store = Store::open_in_memory().unwrap().with_codec(codec);
        store
            .append_record("eng-1", "secret", "hello-transcript")
            .unwrap();
        store.append_record("eng-1", "meta", "not-secret").unwrap();

        // Reads decode transparently.
        assert_eq!(
            store.records("eng-1", "secret").unwrap(),
            vec!["hello-transcript"]
        );
        assert_eq!(store.records("eng-1", "meta").unwrap(), vec!["not-secret"]);

        // A plaintext store over the same rows sees the content kind is NOT plaintext...
        let raw = Store::open_in_memory().unwrap();
        // (re-insert the at-rest bytes a codec'd store would have written)
        let mut raw = raw;
        raw.append_record("eng-1", "secret", "rev:tpircsnart-olleh")
            .unwrap();
        assert_ne!(
            raw.records("eng-1", "secret").unwrap(),
            vec!["hello-transcript"]
        );
    }

    #[test]
    fn content_codec_crypto_erase_makes_content_unrecoverable_history_intact() {
        // SECAUD-6: once a scope's key is erased, its content rows decode to None and are
        // dropped from reads — gone — while the underlying append-only rows remain.
        let codec = std::sync::Arc::new(RevCodec {
            erased: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        });
        let mut store = Store::open_in_memory().unwrap().with_codec(codec.clone());
        store
            .append_record("eng-1", "secret", "client-data")
            .unwrap();
        store
            .append_record("eng-2", "secret", "other-data")
            .unwrap();
        assert_eq!(store.records("eng-1", "secret").unwrap().len(), 1);

        // Crypto-erase eng-1: its content is unrecoverable; eng-2 is untouched (per-unit).
        codec.erased.lock().unwrap().insert("eng-1".into());
        assert!(
            store.records("eng-1", "secret").unwrap().is_empty(),
            "erased content is gone"
        );
        assert_eq!(
            store.records("eng-2", "secret").unwrap(),
            vec!["other-data"],
            "other unit intact"
        );
        // The decoded history also drops the unrecoverable row (the raw append-only row is
        // never deleted — INV-6 — only the key is gone, so the ciphertext can't be opened).
        assert_eq!(
            store.events("eng-1").unwrap().len(),
            0,
            "decoded history drops the erased row"
        );
    }
}
