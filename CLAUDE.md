# MeshTrove

Self-hosted Printables-style archive for downloaded and purchased 3D models.
Rust/Axum/SQLx backend + React/TS/MUI frontend, single binary serves both.
Design doc: `docs/plan.md` (ER diagram + rationale); decisions: `docs/decisions.md`.

## Run (development)

```bash
docker compose up -d                 # postgres:17 on :5432
cp .env.example .env                 # defaults are dev-ready (anonymous admin)
cd frontend && npm install && npm run dev   # Vite on :5173
cd backend && cargo run              # http://localhost:3001 (proxies to Vite)
```

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
- Variant attributes (scale, support, …) are data-driven rows in
  `variant_axes` / `variant_axis_options` — never add hard-coded enums/columns
  for them.
- Descriptions are immutable revisions (newest = current); edits insert.
- Big uploads stream end-to-end; don't buffer whole files in memory.

## Verify

`cargo test && cargo clippy` in backend; `npx tsc -b && npm run build` in
frontend. End-to-end: upload a zip to a variant, watch `/api/jobs`, confirm
files keep their folder structure and a preview renders (needs `f3d` on PATH).
