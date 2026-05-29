//! ol — OpenLedger CLI. (AGPL-3.0)
//!
//! Stage 0 placeholder dispatcher (no deps). Stage 1 wires these to ol-api /
//! ol-ledger / ol-events and adds a real arg parser.

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "serve" => println!("ol serve: start MCP + REST server (Stage 1)"),
        "migrate" => println!("ol migrate: run SQL migrations (Stage 1)"),
        "replay" => println!("ol replay: rebuild projections from the event log (Stage 1)"),
        "post" => println!("ol post: post a journal entry (Stage 1)"),
        other => {
            if !other.is_empty() {
                eprintln!("ol: unknown command '{other}'");
            }
            eprintln!("usage: ol <serve|migrate|replay|post>");
            std::process::exit(2);
        }
    }
}
