//! The chat surface must not be able to double-post (#86).
//!
//! Reproduced live on 2026-08-14: two identical `POST /chat` calls produced
//! journal entries 5 and 6 with two different random keys, because the handler
//! minted `Uuid::new_v4()` per post. The key is now DERIVED, so a retry
//! re-derives it and the ledger's existing dedup engages.
//!
//! These tests drive `dispatch_tool` directly — the Ollama loop is
//! non-deterministic and needs a daemon, but the key derivation is not.

use ol_api::chat::{ToolContext, dispatch_tool};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

fn post_args(memo: &str) -> serde_json::Value {
    json!({
        "journal_code": "GEN",
        "entry_date": "2026-08-14",
        "memo": memo,
        "lines": [
            { "account_code": "1000", "debit": 5000, "credit": 0 },
            { "account_code": "4000", "debit": 0, "credit": 5000 }
        ]
    })
}

async fn entry_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM journal_entries")
        .fetch_one(pool)
        .await
        .expect("count journal_entries")
}

/// GIVEN a chat tool loop,
/// WHEN `post_journal_entry` is dispatched twice with identical args in the same
/// turn,
/// THEN exactly one journal entry exists and the second reports `replayed = true`.
///
/// This is the live double-post path: the model misreads a success as a failure
/// and re-issues on a later loop iteration. The turn index must NOT advance
/// between iterations, or the retry would get a fresh key and post again.
#[sqlx::test(migrations = "../../migrations")]
async fn identical_tool_calls_in_one_turn_post_once(pool: PgPool) {
    let ctx = ToolContext {
        conversation_id: Uuid::new_v4(),
        turn: 0,
    };
    let args = post_args("chat dedup");

    let first = dispatch_tool(&pool, "post_journal_entry", &args, &ctx)
        .await
        .expect("first post must succeed");
    let second = dispatch_tool(&pool, "post_journal_entry", &args, &ctx)
        .await
        .expect("the retry must succeed, not error");

    assert_eq!(
        first["replayed"],
        json!(false),
        "the first post is not a replay"
    );
    assert_eq!(
        second["replayed"],
        json!(true),
        "the retry must be a replay"
    );
    assert_eq!(
        second["entry_id"], first["entry_id"],
        "the replay must return the ORIGINAL entry"
    );
    assert_eq!(entry_count(&pool).await, 1, "exactly one entry must exist");
}

/// GIVEN two genuinely different posts in one conversation,
/// WHEN both are dispatched in the same turn,
/// THEN two entries exist.
///
/// Guards the opposite failure: a key so stable that distinct posts collapse.
#[sqlx::test(migrations = "../../migrations")]
async fn two_different_posts_in_one_turn_both_land(pool: PgPool) {
    let ctx = ToolContext {
        conversation_id: Uuid::new_v4(),
        turn: 0,
    };

    dispatch_tool(&pool, "post_journal_entry", &post_args("first"), &ctx)
        .await
        .expect("first post");
    dispatch_tool(&pool, "post_journal_entry", &post_args("second"), &ctx)
        .await
        .expect("second post");

    assert_eq!(
        entry_count(&pool).await,
        2,
        "different args, different entries"
    );
}

/// GIVEN the same args in a LATER turn of the same conversation,
/// WHEN dispatched,
/// THEN a second entry is created.
///
/// A user who deliberately repeats a posting in a new turn means it. This is why
/// the key derives from the turn index and not from the conversation alone.
#[sqlx::test(migrations = "../../migrations")]
async fn the_same_post_in_a_later_turn_creates_a_second_entry(pool: PgPool) {
    let conversation_id = Uuid::new_v4();
    let args = post_args("deliberate repeat");

    dispatch_tool(
        &pool,
        "post_journal_entry",
        &args,
        &ToolContext {
            conversation_id,
            turn: 0,
        },
    )
    .await
    .expect("turn 0 post");
    dispatch_tool(
        &pool,
        "post_journal_entry",
        &args,
        &ToolContext {
            conversation_id,
            turn: 1,
        },
    )
    .await
    .expect("turn 1 post");

    assert_eq!(
        entry_count(&pool).await,
        2,
        "a later turn is a new intention"
    );
}

/// GIVEN two different conversations issuing byte-identical tool calls,
/// WHEN both are dispatched at the same turn index,
/// THEN two entries exist — one user's posting never deduplicates another's.
#[sqlx::test(migrations = "../../migrations")]
async fn identical_calls_in_different_conversations_do_not_collide(pool: PgPool) {
    let args = post_args("same text, different people");

    for _ in 0..2 {
        dispatch_tool(
            &pool,
            "post_journal_entry",
            &args,
            &ToolContext {
                conversation_id: Uuid::new_v4(),
                turn: 0,
            },
        )
        .await
        .expect("post");
    }

    assert_eq!(
        entry_count(&pool).await,
        2,
        "distinct conversations must not share keys"
    );
}

/// GIVEN the key derivation,
/// WHEN it is called twice with the same inputs,
/// THEN it returns the same UUID, and differs when any component differs.
///
/// The property the whole fix rests on, asserted directly rather than inferred
/// from row counts.
#[test]
fn key_derivation_is_stable_and_component_sensitive() {
    let conversation_id = Uuid::new_v4();
    let ctx = ToolContext {
        conversation_id,
        turn: 0,
    };
    let args = post_args("x");

    assert_eq!(
        ctx.idempotency_key("post_journal_entry", &args),
        ctx.idempotency_key("post_journal_entry", &args),
        "identical inputs must derive an identical key"
    );

    let other_turn = ToolContext {
        conversation_id,
        turn: 1,
    };
    assert_ne!(
        ctx.idempotency_key("post_journal_entry", &args),
        other_turn.idempotency_key("post_journal_entry", &args),
        "the turn index must change the key"
    );
    assert_ne!(
        ctx.idempotency_key("post_journal_entry", &args),
        ctx.idempotency_key("post_journal_entry", &post_args("y")),
        "different args must change the key"
    );
    assert_ne!(
        ctx.idempotency_key("post_journal_entry", &args),
        ctx.idempotency_key("receive_stock", &args),
        "the tool name must change the key"
    );
}
