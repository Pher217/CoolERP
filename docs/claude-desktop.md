# Driving CoolERP from Claude Desktop

CoolERP's headline capability is that an **AI agent is a first-class operator of
the ledger** — not by screen-scraping a UI, but through a typed MCP capability
surface where the engine tells the agent what it is *legally allowed to do next*
and the database refuses anything that would unbalance the books.

This guide wires Claude Desktop to a local `ol-mcp` server.

> **Security:** `ol-mcp` currently runs **unauthenticated** and is intended for
> local development only. Scoped OAuth is designed in
> [ADR-006](adr/0006-mcp-auth-cimd-over-rfc7591.md) but not yet wired. See
> [SECURITY.md](../SECURITY.md) before exposing anything.

## 1. Start the database and seed demo data

```bash
docker compose up -d db
make demo
```

## 2. Build the MCP server

```bash
cargo build -p ol-mcp
```

## 3. Point Claude Desktop at it

Add this to `claude_desktop_config.json`:

- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "coolerp": {
      "command": "/absolute/path/to/CoolERP/target/debug/ol-mcp",
      "env": {
        "DATABASE_URL": "postgres://coolerp:coolerp@localhost:5432/coolerp",
        "PROCESSES_DIR": "/absolute/path/to/CoolERP/processes"
      }
    }
  }
}
```

Both paths must be **absolute** — Claude Desktop does not expand `~` or resolve
relative paths. Restart Claude Desktop after editing the file.

## 4. The capability surface

Once connected, eight tools are available:

| Tool | What it does |
|---|---|
| `list_processes` | List all business process definitions |
| `get_process` | Get one definition, including its Mermaid state diagram |
| `start_process` | Start a new process instance |
| `get_process_instance` | Get an instance, its full step audit log, **and the `available` next capabilities** |
| `list_process_instances` | List instances, filterable by process and status |
| `advance_process` | Advance an instance by one transition |
| `post_journal_entry` | Post a balanced journal entry, idempotently |
| `get_account_balance` | Read the signed balance of an account |

## 5. The demo worth showing

Ask Claude Desktop:

> Start an `order_to_cash` process, then walk it all the way to cash received.
> Before each step, tell me which transitions are legal and why.

What makes this different from an ERP with a chat bot bolted on:

1. **The agent discovers its own permissions.** `get_process_instance` returns an
   `available` array — the legal capability names from the *current* state. The
   agent is told what it may do; it does not guess and get rejected.
2. **The engine, not the model, decides what is legal.** The state machine lives
   in version-controlled YAML under `processes/`. The LLM proposes; the
   deterministic core disposes ([ADR-009](adr/0009-rust-core-polyglot-edges.md)).
3. **The books cannot go wrong.** Posting transitions go through the same
   `post_journal_entry` path as every other client, and the balance invariant is
   enforced by database triggers — not by the agent, and not by the API layer
   ([ADR-007](adr/0007-money-integer-cents.md),
   [ADR-015](adr/0015-materialized-balance-cache.md)).
4. **Every step is audited.** The instance carries a full step log; entries are
   append-only at the privilege layer
   ([ADR-021](adr/0021-least-privilege-db-role.md)).

Run `make api` and `make ui` alongside it, and the ledger and trial balance in
the web UI update as the agent posts.

## Verifying the server without Claude Desktop

`ol-mcp` speaks MCP over stdio, so you can drive it directly:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | PROCESSES_DIR="$PWD/processes" ./target/debug/ol-mcp
```

That prints the `initialize` result followed by the full tool list — the fastest
way to confirm the server is healthy before debugging a client.
