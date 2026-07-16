# OpenERP developer quickstart. `make demo` takes a clean clone to a running,
# seeded API in one command (Docker + Rust toolchain required).
#
#   make demo    # db + migrate + seed + print next steps
#   make api     # run the REST API (needs db + migrate)
#   make ui      # run the web UI (separate terminal)
#   make test    # cargo test --all against the dev db
#   make verify  # adversarial proof of the append-only privilege boundary
#   make down    # stop the database (keeps data); make clean also drops the volume

export DATABASE_URL ?= postgres://openerp:openerp@localhost:5432/openerp

.PHONY: db migrate seed demo api ui ui-install test verify down clean

db: ## start postgres and wait for health
	docker compose up -d db
	@echo "waiting for postgres..."
	@until docker compose exec -T db pg_isready -U openerp -d openerp >/dev/null 2>&1; do sleep 1; done
	@echo "postgres ready on $${OL_DB_PORT:-5432}"

migrate: db ## apply all migrations (schema + invariants + least-privilege role)
	cargo run -q -p ol-cli -- migrate

seed: ## load balanced demo data so the ledger views show something
	psql "$(DATABASE_URL)" -v ON_ERROR_STOP=1 -f scripts/seed_demo.sql

demo: migrate seed ## one command: db + migrate + seed
	@echo ""
	@echo "  ✔ database ready and seeded."
	@echo "  next:  make api      # REST API + OpenAPI at http://127.0.0.1:3000"
	@echo "         make ui       # web UI (separate terminal)"
	@echo ""

api: ## run the REST API (BIND_ADDR defaults to localhost — no auth yet)
	cargo run -p ol-api

ui-install:
	cd apps/ol-ui && npm install

ui: ## run the web UI dev server
	cd apps/ol-ui && npm run dev

test: ## run the full Rust test suite against the dev db
	cargo test --all

verify: ## prove the append-only privilege boundary holds (migration 0008)
	bash scripts/verify_append_only.sh

down: ## stop postgres, keep data
	docker compose down

clean: ## stop postgres and DROP the data volume
	docker compose down -v
