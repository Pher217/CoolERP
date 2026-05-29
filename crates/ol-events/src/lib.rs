//! ol-events — append-only event store + projections / replay.
//!
//! The journal IS the event log; balances are a derived projection. Stage 1:
//! - append to `events` (actor, capability, inputs_hash, entity_id, result_ids);
//! - rebuild balance projections deterministically from the immutable log so a
//!   fresh DB replayed from the same events yields identical balances (smoke #2).
