//! ol-engine — durable process engine.
//!
//! Drives multi-step business-process instances (order-to-cash, purchase-to-pay,
//! etc.) defined as YAML state machines in the `processes/` directory.
//!
//! Every state transition is atomic:
//!   1. Open REPEATABLE READ transaction; lock instance row.
//!   2. Validate the requested capability against the current state.
//!   3. If a `posting_rule` exists: resolve roles → account codes, build balanced
//!      `Lines`, call [`ol_ledger::post_journal_entry`] (idempotent, retriable).
//!      The key is deterministic — concurrent retries dedup in the ledger.
//!   4. Update `process_instances` with a guarded WHERE … AND current_state=$from;
//!      append an immutable row to `process_steps_log`.
//!
//! Money is integer cents (`i64`). No floats. (ADR-007)

use std::{collections::HashMap, env, path::PathBuf};

use chrono::NaiveDate;
use ol_domain::Line;
use ol_ledger::{PostRequest, post_journal_entry};
use ol_process::{CreditTarget, PostingRule, Process};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// What a caller must supply to drive a posting transition, and what that
/// posting will do.
///
/// The two halves are deliberately separate fields (ADR-025): `credit_roles`
/// *describes* the posting, `required_amount_keys` *prescribes* the call.
/// Supply `required_amount_keys` — `"amount"` alone for a single-credit rule,
/// or `"amount"` plus one key per credited role, summing to it, for a
/// multi-credit rule.  Keys outside that set are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostingRequirement {
    /// The account role that is debited.  The caller supplies its total as
    /// key `"amount"` in `amounts`.
    pub debit_role: String,
    /// Every account role this transition credits — always populated, whether
    /// the rule credits one account or several.  Descriptive: it tells the
    /// caller what the posting *does*, not what it must send (ADR-025).
    pub credit_roles: Vec<String>,
    /// Exactly the keys the caller must supply in `amounts`.  Always contains
    /// `"amount"`; contains the credit role names only when the rule credits
    /// more than one account, because a single credit takes the whole debit
    /// total and needs no key of its own (ADR-025).
    pub required_amount_keys: Vec<String>,
}

/// One legal next move from the current state of a process instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableTransition {
    /// The capability name the caller must pass to `advance_instance`.
    pub capability: String,
    /// The state the instance will move to if this transition fires.
    pub to_state: String,
    /// `Some(req)` when this transition posts a GL entry.  The caller must
    /// supply the amounts described in `req`.  `None` for non-posting transitions.
    pub posting: Option<PostingRequirement>,
}

/// A snapshot of a process instance row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: i64,
    pub process: String,
    pub current_state: String,
    pub status: String,
    pub reference: Option<String>,
    pub context: serde_json::Value,
}

/// Result of a successful [`start_instance`] or [`start_instance_with_key`] call.
///
/// `replayed` is true when the call reused an idempotency key that already
/// owned an instance for the same canonical payload.
#[derive(Debug, Clone)]
pub struct StartInstanceOutcome {
    pub instance: Instance,
    pub replayed: bool,
}

/// Input for a single [`advance_instance`] call.
pub struct AdvanceInput {
    /// Named amounts in integer cents. Keys match role names or "amount" for
    /// simple single-credit posting rules.
    pub amounts: HashMap<String, i64>,
    /// JSON patch merged into the instance context (`context || context_patch`).
    pub context_patch: serde_json::Value,
    /// Accounting date for the GL entry.
    pub entry_date: NaiveDate,
}

/// One row of the append-only step audit log.
#[derive(Debug, Clone)]
pub struct StepLog {
    pub id: i64,
    pub instance_id: i64,
    pub from_state: String,
    pub to_state: String,
    pub capability: String,
    pub actor: String,
    pub entry_id: Option<i64>,
    pub payload: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Errors returned by the engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("process '{0}' not found in the processes directory")]
    ProcessNotFound(String),

    #[error("no transition from '{from}' with capability '{capability}'. Available from '{from}': {available}",
        available = if available.is_empty() { "(none — terminal state)".to_string() } else { available.join(", ") }
    )]
    IllegalTransition {
        from: String,
        capability: String,
        /// Human-readable descriptions of the legal next moves, e.g.
        /// `"run_credit_check -> credit_check"` or
        /// `"deliver -> shipped (posts: debit cost_of_goods_sold, credit inventory)"`.
        available: Vec<String>,
    },

    #[error("instance is not active (current status: {0})")]
    InstanceNotActive(String),

    #[error("unknown account role '{0}'")]
    UnknownRole(String),

    /// The caller supplied amounts for a posting step but the map is missing
    /// required keys.  `missing` lists every absent key so the caller can fix
    /// the call in one go without trial-and-error.
    #[error(
        "posting step '{capability}' needs amounts in integer cents, \
         and must include the keys [{required_amount_keys}]: \
         key 'amount' = the debit total for role '{debit_role}'\
         {multi_credit_note}. \
         Missing: [{missing}].",
        required_amount_keys = required_amount_keys.join(", "),
        multi_credit_note = if required_amount_keys.len() > 1 {
            ", and one key per credited role, all summing to 'amount'"
        } else {
            " (the single credited account takes the whole total)"
        },
        missing = missing.join(", ")
    )]
    PostingAmountsRequired {
        capability: String,
        debit_role: String,
        /// Exactly the keys the caller must supply — NOT the credited roles.
        /// A single-credit rule requires only `"amount"` (ADR-025).
        required_amount_keys: Vec<String>,
        missing: Vec<String>,
    },

    #[error("posting amounts unbalanced: debit {debit} != credit {credit}")]
    Unbalanced { debit: i64, credit: i64 },

    /// Two concurrent callers raced to advance the same instance; the loser
    /// should retry the full `advance_instance` call.
    #[error("concurrent advance conflict: another caller already advanced this instance")]
    ConcurrentAdvance,

    /// The idempotency key was already used for a different payload.
    #[error("IDEMPOTENCY_KEY_REUSED: {0}")]
    IdempotencyKeyReused(String),

    #[error("ledger error: {0}")]
    Ledger(#[from] ol_ledger::PostError),

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("process load error: {0}")]
    ProcessLoad(String),
}

/// Raw DB row for a step-log SELECT (avoids the clippy type_complexity lint).
type StepLogRow = (
    i64,
    String,
    String,
    String,
    String,
    Option<i64>,
    Option<Value>,
    chrono::DateTime<chrono::Utc>,
);

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve the processes directory: PROCESSES_DIR env var → fallback "../../processes"
/// relative to CARGO_MANIFEST_DIR (test) or the binary's cwd (prod).
fn processes_dir() -> PathBuf {
    if let Ok(dir) = env::var("PROCESSES_DIR") {
        return PathBuf::from(dir);
    }
    // In tests `cargo test` sets CARGO_MANIFEST_DIR; in prod fall back to cwd.
    let base = env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base.join("../../processes")
}

/// Load all processes and return the one with the matching name.
fn load_process(name: &str) -> Result<Process, EngineError> {
    let dir = processes_dir();
    let processes = Process::load_dir(&dir).map_err(|e| EngineError::ProcessLoad(e.to_string()))?;
    processes
        .into_iter()
        .find(|p| p.process == name)
        .ok_or_else(|| EngineError::ProcessNotFound(name.to_string()))
}

/// Determine the initial state: the state that never appears as a `to` target.
/// That makes it a source-only state (the entry point of the machine).
fn initial_state(process: &Process) -> String {
    let all_to: std::collections::HashSet<&str> =
        process.transitions.iter().map(|t| t.to.as_str()).collect();
    process
        .states
        .iter()
        .find(|s| !all_to.contains(s.as_str()))
        .cloned()
        .unwrap_or_else(|| process.states[0].clone())
}

/// Return true if `state` has at least one outgoing transition.
fn has_outgoing(process: &Process, state: &str) -> bool {
    process.transitions.iter().any(|t| t.from == state)
}

/// Map a database row to an [`Instance`].
fn row_to_instance(
    id: i64,
    process: String,
    current_state: String,
    status: String,
    reference: Option<String>,
    context: Value,
) -> Instance {
    Instance {
        id,
        process,
        current_state,
        status,
        reference,
        context,
    }
}

/// Return all transitions that are legal from `state` in the given process.
///
/// Each entry in the returned vec describes one legal next move: the capability
/// name to pass, the target state, and (for posting transitions) the amounts
/// the caller must supply.
/// The account roles a posting rule credits, single or multiple alike.
fn credit_roles_of(rule: &PostingRule) -> Vec<String> {
    match &rule.credit {
        CreditTarget::Single(r) => vec![r.clone()],
        CreditTarget::Multiple(rs) => rs.clone(),
    }
}

/// The `amounts` keys a caller must supply for a rule crediting `credit_roles`.
///
/// A single credit takes the entire debit total, so `"amount"` alone suffices.
/// Several credits must each be named, and they must sum to `"amount"`.
fn required_amount_keys(credit_roles: &[String]) -> Vec<String> {
    let mut keys = vec!["amount".to_string()];
    if credit_roles.len() > 1 {
        keys.extend(credit_roles.iter().cloned());
    }
    keys
}

/// Merge `patch` into `base` the same way PostgreSQL `jsonb ||` merges objects:
/// top-level keys from `patch` replace those in `base`.
fn merge_context(base: &Value, patch: &Value) -> Value {
    let mut merged = base.clone();
    if let Some(merged_obj) = merged.as_object_mut() {
        if let Some(patch_obj) = patch.as_object() {
            for (key, value) in patch_obj {
                merged_obj.insert(key.clone(), value.clone());
            }
        }
    } else if patch.is_object() {
        merged = patch.clone();
    }
    merged
}

/// Evaluate a single transition guard against the instance context.
///
/// Guards fail closed: a guard whose inspected field is absent refuses the
/// transition with a clear reason, exactly as it does when the field is present
/// and violates the rule.
///
/// The `lines_nonempty` and `totals_balance` guards are defined for invoice
/// posting transitions that target the `posted` state. On transitions whose
/// target state is not `posted` they are treated as not applicable and pass,
/// because the YAML author may declare them on other invoice-like transitions
/// (e.g. order-to-cash `shipped -> invoiced`) that do not carry invoice-line
/// context. Unknown guards are always refused (fail-closed).
///
/// On failure the reason string is returned so the caller can surface it in an
/// [`EngineError::IllegalTransition`] (the only client-visible error shape that
/// does not require extending the public enum and therefore touching every
/// downstream match site).
fn evaluate_guard(guard: &str, context: &Value, to_state: &str) -> Result<(), String> {
    match guard {
        "lines_nonempty" => {
            // Invoice-line guards only apply to actual invoice-posting transitions.
            if to_state != "posted" {
                return Ok(());
            }
            match context.get("lines") {
                None => Err("context.lines is missing".to_string()),
                Some(Value::Array(lines)) if !lines.is_empty() => Ok(()),
                Some(Value::Array(_)) => Err("context.lines is empty".to_string()),
                Some(_) => Err("context.lines is not an array".to_string()),
            }
        }
        "totals_balance" => {
            if to_state != "posted" {
                return Ok(());
            }
            let lines = match context.get("lines") {
                None => return Err("context.lines is missing".to_string()),
                Some(v) => v,
            };
            let total = match context.get("total") {
                None => return Err("context.total is missing".to_string()),
                Some(v) => v,
            };

            let lines = lines
                .as_array()
                .ok_or_else(|| "context.lines is not an array".to_string())?;
            let total = total
                .as_i64()
                .ok_or_else(|| "context.total is not an integer number of cents".to_string())?;

            let mut sum: i64 = 0;
            for (idx, line) in lines.iter().enumerate() {
                let amount = line
                    .get("amount")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| format!("context.lines[{idx}] has no integer amount"))?;
                sum = sum
                    .checked_add(amount)
                    .ok_or_else(|| "sum of line amounts overflows".to_string())?;
            }

            if sum == total {
                Ok(())
            } else {
                Err(format!(
                    "line amounts sum to {sum}, context.total is {total}"
                ))
            }
        }
        _ => Err("unknown guard".to_string()),
    }
}

pub fn available_transitions(proc: &Process, state: &str) -> Vec<AvailableTransition> {
    proc.transitions
        .iter()
        .filter(|t| t.from == state)
        .filter_map(|t| {
            // Skip transitions that have no capability name (internal/auto transitions).
            let capability = t.capability.as_ref()?.clone();

            let posting = t.posting_rule.as_ref().map(|rule| {
                let credit_roles = credit_roles_of(rule);
                PostingRequirement {
                    required_amount_keys: required_amount_keys(&credit_roles),
                    debit_role: rule.debit.clone(),
                    credit_roles,
                }
            });

            Some(AvailableTransition {
                capability,
                to_state: t.to.clone(),
                posting,
            })
        })
        .collect()
}

/// Build the human-readable summary strings used in `IllegalTransition::available`.
///
/// Example outputs:
///   "run_credit_check -> credit_check"
///   "deliver -> shipped (posts: debit cost_of_goods_sold, credit inventory)"
///   "post_invoice -> invoiced (posts: debit accounts_receivable, credit sales_revenue, tax_payable)"
fn available_transition_hints(proc: &Process, state: &str) -> Vec<String> {
    available_transitions(proc, state)
        .into_iter()
        .map(|at| {
            if let Some(p) = &at.posting {
                format!(
                    "{} -> {} (posts: debit {}, credit {})",
                    at.capability,
                    at.to_state,
                    p.debit_role,
                    p.credit_roles.join(", ")
                )
            } else {
                format!("{} -> {}", at.capability, at.to_state)
            }
        })
        .collect()
}

/// Derive a deterministic UUIDv5 idempotency key for a posting step.
///
/// The key embeds the instance id, the source state, the capability being
/// executed, and the count of already-completed steps for that instance.
/// Concurrent callers deriving the same key hit the ledger's idempotency
/// dedup rather than double-posting.
fn derive_idempotency_key(
    instance_id: i64,
    from_state: &str,
    capability: &str,
    step_count: i64,
) -> Uuid {
    let s = format!("{instance_id}:{from_state}:{capability}:{step_count}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, s.as_bytes())
}

/// Operation name recorded in `idempotency_keys.operation` for instance starts.
const START_INSTANCE_OPERATION: &str = "start_instance";

/// Retries for transient idempotency-key races during instance starts.
const MAX_ATTEMPTS: u32 = 10;

/// Exponential backoff with attempt-derived jitter (no wall-clock / RNG needed).
async fn backoff(attempt: u32) {
    let base_ms = 2_u64.saturating_pow(attempt).min(256);
    let jitter_ms = u64::from(attempt) * 3;
    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
}

/// Canonical SHA-256 request hash for starting an instance.
///
/// Includes the idempotency key itself, so reusing the key with a different
/// payload produces a different hash and fails closed. The payload is the
/// economically meaningful start input: process, reference, and context.
fn start_instance_request_hash(
    idempotency_key: Uuid,
    process: &str,
    reference: Option<&str>,
    context: &Value,
) -> String {
    #[derive(Serialize)]
    struct Canonical<'a> {
        v: &'static str,
        op: &'static str,
        key: String,
        process: &'a str,
        reference: Option<&'a str>,
        context: &'a Value,
    }

    let canonical = Canonical {
        v: "1",
        op: START_INSTANCE_OPERATION,
        key: idempotency_key.to_string(),
        process,
        reference,
        context,
    };
    let json = serde_json::to_string(&canonical)
        .unwrap_or_else(|e| panic!("failed to canonicalise start_instance payload: {e}"));
    ol_ledger::sha256_hex(json.as_bytes())
}

/// Deterministic idempotency key for callers that do not supply one.
///
/// The engine's own [`start_instance`] uses this so that calling it twice with
/// the same payload is idempotent. REST callers that omit `idempotency_key`
/// also use it.
pub fn derive_start_instance_key(process: &str, reference: Option<&str>, context: &Value) -> Uuid {
    #[derive(Serialize)]
    struct Material<'a> {
        process: &'a str,
        reference: Option<&'a str>,
        context: &'a Value,
    }

    let material = Material {
        process,
        reference,
        context,
    };
    let json = serde_json::to_string(&material)
        .unwrap_or_else(|e| panic!("failed to serialise start_instance key material: {e}"));
    Uuid::new_v5(&Uuid::NAMESPACE_OID, json.as_bytes())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start a new process instance at the initial state.
///
/// The `process_instances` INSERT and the initial `process_steps_log` INSERT
/// are wrapped in a single transaction so neither is visible without the other.
///
/// Callers that need to know whether the call created a fresh instance or replayed
/// an existing one should use [`start_instance_with_key`].
pub async fn start_instance(
    pool: &PgPool,
    process: &str,
    reference: Option<String>,
    context: Value,
) -> Result<Instance, EngineError> {
    let idempotency_key = derive_start_instance_key(process, reference.as_deref(), &context);
    start_instance_with_key(pool, process, reference, context, idempotency_key)
        .await
        .map(|outcome| outcome.instance)
}

/// Start a new process instance with an explicit caller-supplied idempotency key.
///
/// Re-submitting the same `idempotency_key` with the same process, reference,
/// and context returns the original instance (`replayed = true`). Reusing the
/// key with a different payload returns [`EngineError::IdempotencyKeyReused`].
pub async fn start_instance_with_key(
    pool: &PgPool,
    process: &str,
    reference: Option<String>,
    context: Value,
    idempotency_key: Uuid,
) -> Result<StartInstanceOutcome, EngineError> {
    let proc = load_process(process)?;
    for attempt in 0..MAX_ATTEMPTS {
        match try_start_instance(pool, process, &proc, &reference, &context, idempotency_key).await
        {
            Ok(StartOutcome::Done(outcome)) => return Ok(outcome),
            Ok(StartOutcome::Retry) => {
                backoff(attempt).await;
                continue;
            }
            Err(EngineError::Db(e)) if is_serialization_error(&e) => {
                backoff(attempt).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(EngineError::ConcurrentAdvance)
}

/// Result of one transactional start attempt.
enum StartOutcome {
    Done(StartInstanceOutcome),
    Retry,
}

/// One transactional attempt to start an instance, with idempotency-key handling.
async fn try_start_instance(
    pool: &PgPool,
    process: &str,
    proc: &Process,
    reference: &Option<String>,
    context: &Value,
    idempotency_key: Uuid,
) -> Result<StartOutcome, EngineError> {
    let request_hash =
        start_instance_request_hash(idempotency_key, process, reference.as_deref(), context);
    let initial = initial_state(proc);

    let mut tx = pool.begin().await?;

    // Claim the idempotency key. RETURNING 1 is Some(_) iff we inserted the row.
    let claimed: Option<i32> = sqlx::query_scalar(
        "INSERT INTO idempotency_keys (operation, idempotency_key, request_hash) \
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING RETURNING 1",
    )
    .bind(START_INSTANCE_OPERATION)
    .bind(idempotency_key)
    .bind(&request_hash)
    .fetch_optional(&mut *tx)
    .await?;

    if claimed.is_none() {
        // Key already exists: this is a replay. Read with a fresh snapshot, not
        // inside the transaction whose REPEATABLE READ snapshot may predate the
        // owning transaction's commit.
        let _ = tx.rollback().await;
        let stored: Option<(Option<String>, Option<serde_json::Value>)> = sqlx::query_as(
            "SELECT request_hash, result FROM idempotency_keys \
             WHERE operation = $1 AND idempotency_key = $2",
        )
        .bind(START_INSTANCE_OPERATION)
        .bind(idempotency_key)
        .fetch_optional(pool)
        .await?;

        return match stored {
            // Same payload as the stored request: replay the stored instance.
            Some((Some(stored_hash), Some(json))) if stored_hash == request_hash => {
                let instance: Instance = serde_json::from_value(json)
                    .map_err(|e| EngineError::Db(sqlx::Error::Decode(Box::new(e))))?;
                Ok(StartOutcome::Done(StartInstanceOutcome {
                    instance,
                    replayed: true,
                }))
            }
            // Same payload, but the owner has not committed its result yet — retry.
            Some((Some(stored_hash), None)) if stored_hash == request_hash => {
                Ok(StartOutcome::Retry)
            }
            // Different payload, or a legacy row with no recorded hash: fail closed.
            Some((Some(_), _)) | Some((None, _)) => {
                Err(EngineError::IdempotencyKeyReused(format!(
                    "idempotency key {} already used with a different payload",
                    idempotency_key
                )))
            }
            // Owner still in flight — retry.
            None => Ok(StartOutcome::Retry),
        };
    }

    // We own this start. Create the instance and record the result in the same
    // transaction so a replay never sees a row with a NULL result.
    let (id, current_state, status): (i64, String, String) = sqlx::query_as(
        "INSERT INTO process_instances (process, current_state, status, reference, context) \
         VALUES ($1, $2, 'active', $3, $4) \
         RETURNING id, current_state, status::text",
    )
    .bind(process)
    .bind(&initial)
    .bind(reference)
    .bind(context)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO process_steps_log \
         (instance_id, from_state, to_state, capability, actor, entry_id, payload) \
         VALUES ($1, '', $2, 'start', 'system', NULL, NULL)",
    )
    .bind(id)
    .bind(&initial)
    .execute(&mut *tx)
    .await?;

    let instance = row_to_instance(
        id,
        process.to_string(),
        current_state,
        status,
        reference.clone(),
        context.clone(),
    );
    let instance_json = serde_json::to_value(&instance)
        .map_err(|e| EngineError::Db(sqlx::Error::Encode(Box::new(e))))?;

    sqlx::query(
        "UPDATE idempotency_keys SET result = $3 \
         WHERE operation = $1 AND idempotency_key = $2",
    )
    .bind(START_INSTANCE_OPERATION)
    .bind(idempotency_key)
    .bind(instance_json)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(StartOutcome::Done(StartInstanceOutcome {
        instance,
        replayed: false,
    }))
}

/// True for serialization-class failures that merit a retry.
fn is_serialization_error(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::Database(db)
            if db.code().as_deref() == Some("40001") || db.code().as_deref() == Some("40P01")
    )
}

/// Advance a process instance by one state transition.
///
/// Critical order of operations (money-safe):
///
/// 1. Open REPEATABLE READ transaction; lock the instance row FOR UPDATE.
/// 2. Validate the requested capability; reject if not active or no transition.
/// 3. Count existing step-log rows to derive a deterministic idempotency key.
/// 4. If a `posting_rule` exists: resolve the debit account's currency, build
///    balanced Lines, and call `post_journal_entry` (idempotent via key). The
///    ledger manages its own transaction; we call it while our lock is still open.
/// 5. Guarded UPDATE: `WHERE id=$1 AND current_state=$from AND status='active'`.
///    `rows_affected==0` means idempotent success (already at `$to`) or `ConcurrentAdvance`.
/// 6. INSERT step log; COMMIT.
pub async fn advance_instance(
    pool: &PgPool,
    instance_id: i64,
    capability: &str,
    actor: &str,
    input: AdvanceInput,
) -> Result<Instance, EngineError> {
    let mut tx = pool.begin().await?;

    // REPEATABLE READ gives a stable snapshot; FOR UPDATE prevents concurrent
    // advances from reading stale current_state.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;

    // ── Step 1: Lock the instance row ────────────────────────────────────────
    let row: Option<(String, String, String, Option<String>, Value)> = sqlx::query_as(
        "SELECT process, current_state, status::text, reference, context \
         FROM process_instances \
         WHERE id = $1 \
         FOR UPDATE",
    )
    .bind(instance_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (process_name, current_state, status, reference, context) =
        row.ok_or(EngineError::Db(sqlx::Error::RowNotFound))?;

    if status != "active" {
        tx.rollback().await?;
        return Err(EngineError::InstanceNotActive(status));
    }

    // ── Step 2: Find the matching transition ─────────────────────────────────
    let proc = load_process(&process_name)?;

    let transition = proc
        .transitions
        .iter()
        .find(|t| t.from == current_state && t.capability.as_deref() == Some(capability))
        .ok_or_else(|| EngineError::IllegalTransition {
            from: current_state.clone(),
            capability: capability.to_string(),
            available: available_transition_hints(&proc, &current_state),
        })?
        .clone();

    // ── Step 2b: Evaluate declared transition guards ─────────────────────────
    // Guards run against the context as it will be after applying the patch,
    // before any GL post or state change is persisted. A refused guard is
    // surfaced as an IllegalTransition because the transition exists but cannot
    // fire for this context.
    let merged_context = merge_context(&context, &input.context_patch);
    if let Some(guards) = &transition.guards {
        for guard in guards {
            if let Err(reason) = evaluate_guard(guard, &merged_context, &transition.to) {
                tx.rollback().await?;
                return Err(EngineError::IllegalTransition {
                    from: current_state.clone(),
                    capability: capability.to_string(),
                    available: vec![format!("guard '{guard}' refused: {reason}")],
                });
            }
        }
    }

    // ── Step 3: Derive deterministic idempotency key ─────────────────────────
    // Count existing step-log rows for this instance so the key is unique per
    // transition attempt and stable across concurrent retries for the same step.
    let step_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM process_steps_log WHERE instance_id = $1")
            .bind(instance_id)
            .fetch_one(&mut *tx)
            .await?;

    // ── Step 4: Posting execution (money-critical) ────────────────────────────
    let entry_id: Option<i64> = if let Some(rule) = &transition.posting_rule {
        // Resolve the debit role to an account code.
        let debit_code: Option<String> =
            sqlx::query_scalar("SELECT account_code FROM account_roles WHERE role = $1")
                .bind(&rule.debit)
                .fetch_optional(&mut *tx)
                .await?;
        let debit_code = debit_code.ok_or_else(|| EngineError::UnknownRole(rule.debit.clone()))?;

        // Fix 5: resolve currency from the debit account instead of hardcoding.
        let currency: String = sqlx::query_scalar("SELECT currency FROM accounts WHERE code = $1")
            .bind(&debit_code)
            .fetch_one(&mut *tx)
            .await?;

        // Resolve credit role(s) to account codes (before reading "amount" so we
        // can build a complete missing-key list in one shot if needed).
        let credit_roles: Vec<String> = credit_roles_of(rule);

        // Compute the set of missing required amount keys up front so the error
        // message names ALL of them rather than just the first one encountered.
        {
            let mut missing: Vec<String> = Vec::new();
            if !input.amounts.contains_key("amount") {
                missing.push("amount".to_string());
            }
            // For multi-credit posting rules the caller must supply every role key.
            if credit_roles.len() > 1 {
                for role in &credit_roles {
                    if !input.amounts.contains_key(role.as_str()) {
                        missing.push(role.clone());
                    }
                }
            }
            if !missing.is_empty() {
                tx.rollback().await?;
                return Err(EngineError::PostingAmountsRequired {
                    capability: capability.to_string(),
                    debit_role: rule.debit.clone(),
                    required_amount_keys: required_amount_keys(&credit_roles),
                    missing,
                });
            }
        }

        let debit_total = *input.amounts.get("amount").expect("checked above");

        let mut credit_lines: Vec<(String, i64)> = Vec::with_capacity(credit_roles.len());
        let mut credit_sum: i64 = 0;

        for role in &credit_roles {
            let code: Option<String> =
                sqlx::query_scalar("SELECT account_code FROM account_roles WHERE role = $1")
                    .bind(role)
                    .fetch_optional(&mut *tx)
                    .await?;
            let code = code.ok_or_else(|| EngineError::UnknownRole(role.clone()))?;

            let role_amount = if credit_roles.len() == 1 {
                // Single credit role: the entire debit goes here, unconditionally.
                //
                // This used to read a same-named key from `amounts` when one
                // happened to be present, falling back to the debit total. That
                // contradicted the advertised contract (ADR-025), which says a
                // single-credit rule needs only "amount": a caller who also sent
                // `{"inventory": 9000}` alongside `{"amount": 10000}` got a
                // confusing Unbalanced error instead of the posting they asked
                // for. Any value other than the debit total is unbalanced by
                // construction here, so honouring the contract loses nothing.
                debit_total
            } else {
                *input.amounts.get(role.as_str()).expect("checked above")
            };

            credit_sum += role_amount;
            credit_lines.push((code, role_amount));
        }

        // Engine pre-check: credits must sum to the debit total.
        if credit_sum != debit_total {
            tx.rollback().await?;
            return Err(EngineError::Unbalanced {
                debit: debit_total,
                credit: credit_sum,
            });
        }

        // Build the balanced `Line` vec.
        let mut lines: Vec<Line> = Vec::with_capacity(1 + credit_lines.len());
        lines.push(Line::in_currency(&debit_code, debit_total, 0, &currency));
        for (code, amount) in &credit_lines {
            lines.push(Line::in_currency(code.as_str(), 0, *amount, &currency));
        }

        // Derive the deterministic idempotency key for this posting step.
        let idempotency_key =
            derive_idempotency_key(instance_id, &current_state, capability, step_count);

        // post_journal_entry manages its own transaction internally (REPEATABLE READ
        // + retry loop). We call it while our lock transaction is still open so
        // the FOR UPDATE lock on process_instances is held for the full duration,
        // preventing a second concurrent caller from reading the same current_state.
        // The key is deterministic so a concurrent caller deriving the same key
        // will hit the ledger's idempotency dedup and return the same entry_id.
        let post_req = PostRequest {
            idempotency_key,
            journal_code: "GEN".to_string(),
            entry_date: input.entry_date,
            effective_date: None,
            memo: Some(format!("{}:{}", process_name, capability)),
            reference: reference.clone(),
            actor: actor.to_string(),
            lines,
        };

        let result = post_journal_entry(pool, &post_req).await?;

        Some(result.entry_id)
    } else {
        None
    };

    // ── Step 5: Guarded instance update ──────────────────────────────────────
    let new_state = &transition.to;
    let new_status = if has_outgoing(&proc, new_state) {
        "active"
    } else {
        "completed"
    };

    // Merge context_patch into existing context.
    let result: Option<(String, String, Value)> = sqlx::query_as(
        "UPDATE process_instances \
         SET current_state = $2, \
             status = $3::instance_status, \
             context = context || $4, \
             updated_at = now() \
         WHERE id = $1 AND current_state = $5 AND status = 'active' \
         RETURNING current_state, status::text, context",
    )
    .bind(instance_id)
    .bind(new_state)
    .bind(new_status)
    .bind(&input.context_patch)
    .bind(&current_state)
    .fetch_optional(&mut *tx)
    .await?;

    let (updated_current_state, updated_status, updated_context) = match result {
        Some(row) => row,
        None => {
            // rows_affected == 0: either the state already advanced (idempotent
            // success) or another caller won the race (concurrent advance).
            tx.rollback().await?;
            let current: Option<(String,)> =
                sqlx::query_as("SELECT current_state FROM process_instances WHERE id = $1")
                    .bind(instance_id)
                    .fetch_optional(pool)
                    .await?;
            match current {
                Some((state,)) if state == *new_state => {
                    // Already at the target state — idempotent success.
                    let result = get_instance(pool, instance_id).await?;
                    return Ok(result.instance);
                }
                _ => return Err(EngineError::ConcurrentAdvance),
            }
        }
    };

    // ── Step 6: Append step log ──────────────────────────────────────────────
    let payload = serde_json::to_value(&input.amounts).ok();

    sqlx::query(
        "INSERT INTO process_steps_log \
         (instance_id, from_state, to_state, capability, actor, entry_id, payload) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(instance_id)
    .bind(&current_state)
    .bind(new_state)
    .bind(capability)
    .bind(actor)
    .bind(entry_id)
    .bind(&payload)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(row_to_instance(
        instance_id,
        process_name,
        updated_current_state,
        updated_status,
        reference,
        updated_context,
    ))
}

/// Result of [`get_instance`]: the instance snapshot, its full step log, and
/// the legal next transitions from the current state.
pub struct GetInstanceResult {
    pub instance: Instance,
    pub steps: Vec<StepLog>,
    /// Legal next moves from the instance's current state.  Empty when the
    /// instance is in a terminal state (no outgoing transitions).
    pub available: Vec<AvailableTransition>,
}

/// Retrieve a process instance, its full step log, and available next transitions.
pub async fn get_instance(pool: &PgPool, id: i64) -> Result<GetInstanceResult, EngineError> {
    let row: Option<(String, String, String, Option<String>, Value)> = sqlx::query_as(
        "SELECT process, current_state, status::text, reference, context \
         FROM process_instances WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let (process, current_state, status, reference, context) =
        row.ok_or(EngineError::Db(sqlx::Error::RowNotFound))?;

    let instance = row_to_instance(
        id,
        process.clone(),
        current_state.clone(),
        status,
        reference,
        context,
    );

    let log_rows: Vec<StepLogRow> = sqlx::query_as(
        "SELECT id, from_state, to_state, capability, actor, entry_id, payload, created_at \
             FROM process_steps_log \
             WHERE instance_id = $1 \
             ORDER BY id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let steps = log_rows
        .into_iter()
        .map(
            |(log_id, from_state, to_state, capability, actor, entry_id, payload, created_at)| {
                StepLog {
                    id: log_id,
                    instance_id: id,
                    from_state,
                    to_state,
                    capability,
                    actor,
                    entry_id,
                    payload,
                    created_at,
                }
            },
        )
        .collect();

    // Compute available transitions from the current state.
    let available = match load_process(&process) {
        Ok(proc) => available_transitions(&proc, &current_state),
        Err(_) => vec![], // process YAML unavailable; return empty rather than failing
    };

    Ok(GetInstanceResult {
        instance,
        steps,
        available,
    })
}

/// List process instances, optionally filtered by process name and/or status.
pub async fn list_instances(
    pool: &PgPool,
    process: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Instance>, EngineError> {
    let rows: Vec<(i64, String, String, String, Option<String>, Value)> = sqlx::query_as(
        "SELECT id, process, current_state, status::text, reference, context \
         FROM process_instances \
         WHERE ($1::text IS NULL OR process = $1) \
           AND ($2::text IS NULL OR status::text = $2) \
         ORDER BY id",
    )
    .bind(process)
    .bind(status)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, process, current_state, status, reference, context)| {
            row_to_instance(id, process, current_state, status, reference, context)
        })
        .collect())
}
