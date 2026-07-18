# MeshTrove

A self-hosted, Printables-style archive for the 3D models you've downloaded and
purchased. Drop in a zip, keep its folder structure, browse and search your
library, tag variants (32mm, supported, …), and get an automatic preview render
of each model. A single Rust binary serves both the API and the web UI.

- **Backend** — Rust, [Axum](https://github.com/tokio-rs/axum),
  [SQLx](https://github.com/launchbadge/sqlx) over PostgreSQL
- **Frontend** — React + TypeScript + [MUI](https://mui.com), built with Vite
- **Storage** — content-addressed blob store on the filesystem (sha256-keyed);
  logical paths and filenames live in the database, blobs are immutable
- **Previews** — thumbnails rendered by shelling out to [`f3d`](https://f3d.app)

## Features

- **Import, don't lose structure.** A dropped archive stages in an *import* —
  invisible to browse — until you commit it onto a model or a bundle. Files keep
  their original folder layout and filenames.
- **Models and bundles.** A model is one thing; a bundle groups many. Files have
  exactly one owner and never move between the two.
- **Variants by tags.** A variant *is* its set of tags — a flat, data-driven
  vocabulary, not a fixed set of columns. Retagging a variant onto a set the
  model already has merges the two.
- **Full-text search** over your library, backed by Postgres tsvector.
- **Immutable description revisions** — edits insert a new revision; the newest
  is current.
- **Streaming uploads** — large files stream end-to-end; nothing is buffered
  whole in memory.
- **Background jobs** — an in-binary worker (Postgres `SKIP LOCKED`) handles
  imports and renders.

## Quick start (development)

Requires a recent Rust toolchain, Node.js, Docker (for Postgres), and
[`f3d`](https://f3d.app) on your `PATH` for preview rendering.

```bash
docker compose up -d                 # postgres:17 on :5432
cp .env.example .env                 # defaults are dev-ready (anonymous admin)

# one-time: the SQLx query macros validate against a live DB at compile time,
# so the schema must be applied before the first build
cargo install sqlx-cli --no-default-features --features native-tls,postgres
cd backend && sqlx migrate run

# frontend dev server (Vite on :5173)
cd frontend && npm install && npm run dev

# backend — serves the API and proxies to Vite in dev
cd backend && cargo run              # http://localhost:3001
```

> **Migrations must be applied before the first `cargo run`.** The SQLx `query!`
> macros check queries against a live database at *compile* time, so a fresh
> empty DB won't compile. Run them via `sqlx migrate run` (not raw `psql`) so
> they're recorded in `_sqlx_migrations` and the startup `migrate!()` skips them
> instead of re-applying. There is no offline `.sqlx` cache — Postgres must be up
> and migrated for any build.

## Configuration

Every setting is available as both a CLI flag and an environment variable — run
`meshtrove --help` for the full list. `.env` is auto-loaded in development; see
`.env.example` for the annotated defaults.

- `MESHTROVE_ANONYMOUS=true` makes every request a synthetic admin — a dev
  convenience. Unset it to exercise real authentication.
- `MESHTROVE_CREATE_ADMIN=user:pass` bootstraps an admin account at startup.
- `MESHTROVE_COOKIE_KEY` signs session cookies (base64 of ≥64 bytes; required in
  production — sessions are invalidated whenever it changes).

## Production

A multi-stage `Dockerfile` builds the single binary with the frontend baked in.
`docker-compose.prod.yml` runs it alongside Postgres:

```bash
cp .env.prod.example .env.prod       # fill in the secrets — it's git-ignored
docker compose --env-file .env.prod -f docker-compose.prod.yml up -d --build
```

See `docs/deploy.md` for details.

## Project layout

```
backend/
  src/routes/        one file per API area; routers merged in main.rs
  src/services/      blobstore, jobs (SKIP LOCKED worker), importer (zip), renderer (f3d)
  migrations/        full schema, tsvector triggers, seed axes
frontend/
  src/api.ts         typed fetch layer
  src/pages/, src/components/
docs/                plan.md (ER diagram + rationale), decisions.md, deploy.md, …
```

## Development

```bash
# backend
cd backend && cargo test && cargo clippy

# frontend
cd frontend && npx tsc -b && npm run build
```

End-to-end smoke test: upload a zip to a variant, watch `/api/jobs`, and confirm
the files keep their folder structure and a preview renders (needs `f3d`).

## Documentation

- `docs/plan.md` — design doc: ER diagram and rationale
- `docs/decisions.md` — recorded design decisions
- `docs/deploy.md` — production deployment
- `docs/import-layouts.md` — how import layouts map archive contents onto models

## License

BSD 3-Clause — see [LICENSE](LICENSE).
