# MeshTrove

Self-hosted Printables-style archive for downloaded and purchased 3D models.
Rust/Axum/SQLx backend + React/TS/MUI frontend, single binary serves both.
Design doc: `docs/plan.md` (ER diagram + rationale); decisions: `docs/decisions.md`.

## Run (development)

```bash
docker compose up -d                 # postgres:17 on :5432
cp .env.example .env                 # defaults are dev-ready (anonymous admin)
cargo install sqlx-cli --no-default-features --features native-tls,postgres  # once
cd backend && sqlx migrate run       # apply schema BEFORE first build (see below)
cd frontend && npm install && npm run dev   # Vite on :5173
cd backend && cargo run              # http://localhost:3001 (proxies to Vite)
```

The `sqlx migrate run` step is **mandatory before the first `cargo run`**: the
sqlx `query!` macros validate against a live DB at *compile* time, but the
runtime `sqlx::migrate!()` in `main.rs` only applies the schema at startup — so a
fresh empty DB won't compile. Running migrations via sqlx-cli (not raw `psql`)
also records them in `_sqlx_migrations`, so the startup `migrate!()` skips them
instead of erroring on re-apply. There is no `.sqlx` offline cache, so postgres
must be up and migrated for any build.

Bind is **3001** on this machine (3000 is taken by an ssh tunnel). All config is
flag + env dual (`meshtrove --help`); `.env` is auto-loaded. `--anonymous` makes
every request a synthetic admin; unset it to exercise real auth
(`MESHTROVE_CREATE_ADMIN=user:pass` bootstraps an admin).

## Structure

- `backend/src/routes/` — one file per API area; routers merged in `main.rs`
- `backend/src/services/` — blobstore (content-addressed FS store, sha256-keyed),
  jobs (SKIP LOCKED worker in-binary), importer (zip), renderer (f3d shell-out)
- `backend/migrations/0001_initial.sql` — full schema, tsvector triggers, seed axes
- `frontend/src/api.ts` — typed fetch layer; `pages/` + `components/`

## Conventions

- sqlx macros check queries against a live DB: postgres must be up (and
  migrated) to compile. `DATABASE_URL` comes from `.env`.
- Model/bundle files are immutable blobs in `store/ab/cd/<sha256>`; logical
  paths/filenames live only in the `files` table. Never write to `store/` directly.
- A `file` has exactly one owner (`num_nonnulls(model_id, variant_id, bundle_id,
  import_id) = 1`). A dropped archive stages in an **import** — not a model, not
  a bundle, invisible to browse — until `POST /api/imports/{id}/commit` moves its
  files onto one owner. Models and bundles never convert into each other.
- A variant **is its set of variant tags**. `variant_tags` is a flat, data-driven
  vocabulary (32mm, supported, …) — never add hard-coded enums/columns for them,
  and never reuse the model `tags` table, which says what a model *is*. `name` is
  an optional label. Identity lives in the trigger-maintained `tag_key`, unique
  per model: so a model has at most **one anonymous variant** (no name, no tags —
  its plain bucket of files), and retagging a variant onto a tag set the model
  already has **merges** the two rather than erroring.
- Descriptions are immutable revisions (newest = current); edits insert.
- Big uploads stream end-to-end; don't buffer whole files in memory.

## Verify

`cargo test && cargo clippy` in backend; `npx tsc -b && npm run build` in
frontend. End-to-end: upload a zip to a variant, watch `/api/jobs`, confirm
files keep their folder structure and a preview renders (needs `f3d` on PATH).
