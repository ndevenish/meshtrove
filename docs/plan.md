# MeshTrove — Milestone 1: Core Archive

## Context

Greenfield project (repo contains only `docs/spec.md` and `docs/PROJECT_TEMPLATE.md`).
MeshTrove (working title in the spec: "MySTL") is a self-hosted
Printables/Thingiverse-style archive for downloaded **and
purchased** 3D models: one central place for models, their variants, files, print
notes, images, and bundles. Backend is Rust/Axum/SQLx (Postgres, with migrations);
frontend is React/TypeScript/MaterialUI via Vite, structured per
`docs/PROJECT_TEMPLATE.md` (single binary serves prod static files or dev-proxies
to Vite; no CORS; clap config with env/flag duality).

### Decisions made with the user

0. **Name**: **MeshTrove**. Crate/binary `meshtrove`, env-var prefix `MESHTROVE_`,
   default DB name `meshtrove`. (The repo directory is still `mystl`; renaming it
   is up to the user.)

1. **Storage**: content-addressed filesystem blob store (`store/ab/cd/<sha256>`),
   behind a `BlobStore` trait so S3 can be swapped in later. Postgres owns all
   metadata including logical folder structure. Dedup falls out of hash-keying.
2. **Tags vs variants**: two separate systems, unified at the search API. Variants
   are structured children of a model; tags are free-form labels on models/bundles.
   Variant attributes are **not hard-coded columns or enums**: categories ("axes",
   e.g. scale, support) and their options (e.g. 32mm, 75mm; unsupported, supported,
   supported_hollow, lychee_project, merged) are **declarable data** in their own
   tables — users add new axes and options at will; scale/support are only seed
   rows. A search like `tags=egypt,undead & opt=scale:32mm & opt=support:unsupported`
   filters models by tags and variants by their axis options.
3. **Scope**: "core archive first" — the **full** schema lands in migration 0001
   (including jobs, bundles, tags, marks, settings, so it reviews as a whole), but
   this milestone implements: scaffold, auth+roles, blob store, model/variant/file
   upload with zip import, browse/detail UI, tagging, job queue, f3d preview
   rendering. Bundles UI, likes/printed UI, dedup report are fast follow-ups.
4. **Rendering**: shell out to an external tool (f3d first) from a background job —
   no in-project mesh parser/renderer. The renderer command is an **admin-global
   setting**; changing it affects only new renders. Each rendered image records
   the renderer + config that produced it, so an admin can bulk re-render
   "everything still on the previous renderer", choosing **add** (keep old image)
   or **replace**.

> The canonical copy of this plan lives in the repo at `docs/plan.md`.

## Database schema (migration `0001_initial.sql`)

### Entity-relationship diagram

```mermaid
erDiagram
    users {
        uuid id PK
        citext username UK
        text password_hash
        user_role role "admin | editor | viewer"
    }
    creators {
        uuid id PK
        text name
        creator_kind kind "author | company | site"
        text url
    }
    models {
        uuid id PK
        text name
        text slug UK
        uuid creator_id FK
        text source_url
        text license
        numeric purchase_price
        uuid created_by FK
        tsvector search
    }
    model_description_revisions {
        uuid id PK
        uuid model_id FK
        text body_md "markdown"
        citext label "optional name, e.g. v1, v2"
        uuid created_by FK
        timestamptz created_at
    }
    model_variants {
        uuid id PK
        uuid model_id FK
        text name
        uuid derived_from_variant_id FK
        text print_notes
    }
    variant_axes {
        uuid id PK
        citext name UK "scale, support, ... (seed data, user-extendable)"
        text description
        int sort_order
    }
    variant_axis_options {
        uuid id PK
        uuid axis_id FK
        citext value "32mm, unsupported, ... (user-extendable)"
        int sort_order
    }
    variant_options {
        uuid variant_id PK_FK
        uuid axis_id PK_FK "one option per axis per variant"
        uuid option_id FK
    }
    blobs {
        char64 sha256 PK
        bigint size
    }
    files {
        uuid id PK
        char64 blob_sha256 FK
        uuid model_or_variant_or_bundle_id FK "exactly one"
        text path "kept folder structure"
        text filename
        file_kind kind "model | document | archive | other"
    }
    images {
        uuid id PK
        char64 blob_sha256 FK
        uuid model_or_variant_or_bundle_id FK "exactly one"
        image_kind kind "uploaded | imported | rendered"
        uuid source_file_id FK
        text renderer "provenance for re-render"
        jsonb renderer_config
        bool is_primary
    }
    bundles {
        uuid id PK
        text name
        uuid creator_id FK
        bundle_kind kind "purchased | collection"
    }
    tags {
        uuid id PK
        citext name UK
    }
    user_model_marks {
        uuid user_id PK_FK
        uuid model_id PK_FK
        mark_kind mark "liked | printed | wanted"
    }
    jobs {
        bigserial id PK
        text kind "import_archive | render_preview | ..."
        jsonb payload
        job_status status
        timestamptz run_after
    }
    settings {
        text key PK "e.g. renderer"
        jsonb value
    }

    creators ||--o{ models : "made by"
    creators ||--o{ bundles : "made by"
    users ||--o{ models : "created"
    models ||--o{ model_description_revisions : "description history"
    models ||--o{ model_variants : "has"
    model_variants o|--o{ model_variants : "derived_from"
    variant_axes ||--o{ variant_axis_options : "declares"
    model_variants ||--o{ variant_options : "assigned"
    variant_axis_options ||--o{ variant_options : ""
    blobs ||--o{ files : "content"
    blobs ||--o{ images : "content"
    model_variants ||--o{ files : "parts"
    models ||--o{ files : "documents"
    bundles ||--o{ files : "documents"
    models ||--o{ images : ""
    model_variants ||--o{ images : ""
    bundles ||--o{ images : ""
    files o|--o{ images : "rendered from"
    bundles }o--o{ models : "bundle_models"
    bundles }o--o{ bundles : "bundle_children"
    models }o--o{ tags : "model_tags"
    bundles }o--o{ tags : "bundle_tags"
    users }o--o{ models : "user_model_marks"
```

### Table definitions

Postgres, `uuid` PKs (`gen_random_uuid()`), `timestamptz` everywhere, `citext` for
case-insensitive uniques. Enums as Postgres `CREATE TYPE`.

```
-- auth
users            id, username citext UNIQUE, password_hash (argon2id),
                 role user_role ('admin'|'editor'|'viewer'), created_at
                 -- login via signed PrivateCookieJar; no sessions table

-- provenance
creators         id, name, kind creator_kind ('author'|'company'|'site'),
                 url, notes, created_at
                 -- e.g. Loot Studios (company), Printables (site), an author

-- core catalogue
models           id, name, slug UNIQUE, creator_id FK NULL,
                 source_url, license text NULL,
                 purchase_price / purchase_date / order_ref NULL,   -- bought models
                 created_by FK users, created_at, updated_at
                 -- description lives in revisions (below); search tsvector over
                 -- name + current revision body, maintained by trigger + GIN index

-- markdown descriptions with full edit history; every save is a new immutable
-- revision (current = newest), optionally nameable ("v1", "v2")
model_description_revisions
                 id, model_id FK, body_md text, label citext NULL,
                 created_by FK users, created_at
                 UNIQUE (model_id, label) WHERE label IS NOT NULL

model_variants   id, model_id FK, name,
                 derived_from_variant_id FK NULL, -- user-made variants point at origin
                 print_notes text NULL,           -- per-variant print settings/notes
                 created_by, created_at
                 UNIQUE (model_id, name)

-- variant attributes: declarable categories with declarable options (no
-- hard-coded enums — the spec's scale/support examples are just seed data)
variant_axes         id, name citext UNIQUE, description NULL, sort_order,
                     created_by, created_at
variant_axis_options id, axis_id FK, value citext, sort_order, created_by,
                     created_at, UNIQUE (axis_id, value),
                     UNIQUE (axis_id, id)   -- target for composite FK below
variant_options      (variant_id, axis_id) PK,   -- one option per axis per variant
                     option_id,
                     FK (axis_id, option_id) REFERENCES variant_axis_options (axis_id, id)
                     -- composite FK guarantees the option belongs to the axis
-- migration seeds: axis 'scale' (32mm, 75mm) and axis 'support' (unsupported,
-- supported, supported_hollow, lychee_project, merged) — editable/extendable data

-- content-addressed storage
blobs            sha256 char(64) PK, size bigint, created_at

files            id, blob_sha256 FK blobs,
                 model_id / variant_id / bundle_id  -- exactly one non-null (CHECK)
                 path text        -- kept folder structure ('' = root)
                 filename text, mime text,
                 kind file_kind ('model'|'document'|'archive'|'other'),
                 created_at
                 -- variant files = the printable parts; model/bundle files =
                 -- associated documents (stat guides, painting guides, magazines);
                 -- kind='archive' keeps the original uploaded zip for provenance.
                 -- Duplicate discovery = files joined on shared blob_sha256.

images           id, blob_sha256 FK,
                 model_id / variant_id / bundle_id  -- exactly one non-null (CHECK)
                 kind image_kind ('uploaded'|'imported'|'rendered'),
                 source_file_id FK files NULL,      -- what a render was made from
                 renderer text NULL, renderer_config jsonb NULL,  -- provenance for re-render
                 width, height, is_primary bool, sort_order, created_by, created_at

-- bundles (purchasable packs AND personal uber-bundles)
bundles          id, name, slug, description, creator_id FK NULL, source_url,
                 kind bundle_kind ('purchased'|'collection'), created_by, timestamps
bundle_models    (bundle_id, model_id) PK
bundle_children  (parent_bundle_id, child_bundle_id) PK, CHECK parent<>child

-- tagging
tags             id, name citext UNIQUE
model_tags       (model_id, tag_id) PK
bundle_tags      (bundle_id, tag_id) PK

-- user marks (schema now, UI follow-up)
user_model_marks (user_id, model_id, mark mark_kind ('liked'|'printed'|'wanted')) PK,
                 notes NULL, created_at

-- background jobs
jobs             id bigserial, kind text, payload jsonb,
                 status job_status ('queued'|'running'|'succeeded'|'failed'|'cancelled'),
                 priority int, attempts int, max_attempts int, last_error text,
                 run_after timestamptz, started_at, finished_at, created_at
                 INDEX (status, priority, run_after)

-- admin-global settings
settings         key text PK, value jsonb, updated_at, updated_by
                 -- e.g. 'renderer' → {"tool":"f3d","args":[…]}
```

## Search design (unified text + tags + variant options)

One endpoint (`GET /api/models?q=&tags=&opt=axis:value…`) resolved by a single
SQL query in Postgres — no external search engine.

- **Full text**: a `search tsvector` column on `models`, GIN-indexed, maintained
  by triggers and built with weights from everything a user would call "the
  model's text": name (weight A), tag names (B), creator name (B), and the
  **current** description revision's markdown (C, tags stripped). Triggers on
  `models`, `model_description_revisions` (insert = new current), `model_tags`,
  and `creators` keep it fresh; no application code has to remember to reindex.
  Queries use `websearch_to_tsquery('english', $q)` (supports quoted phrases,
  `-exclusions`) and rank with `ts_rank`.
- **Fuzzy/prefix match**: `pg_trgm` GIN index on `models.name` (and `tags.name`)
  OR-ed in, so misspellings ("anubus") and substring hits still match; also
  powers typeahead for tag and creator comboboxes.
- **Tags**: AND semantics — each requested tag becomes an `EXISTS` against
  `model_tags` (names resolved case-insensitively via `citext`).
- **Variant options**: each `opt=axis:value` pair filters via a single `EXISTS`
  requiring **one variant that satisfies all requested pairs at once** (a model
  with a 32mm-supported and a 75mm-unsupported variant does NOT match
  `32mm + unsupported`); the response marks which variants matched so the UI can
  highlight them.
- All filters compose as `AND` in one statement; ordered by `ts_rank` when `q`
  is present, else `updated_at DESC`; paginated. Filter-sidebar counts come from
  a grouped facet query over the same predicates.

## Architecture

Follow `docs/PROJECT_TEMPLATE.md` closely:

```
backend/
  src/main.rs          logging, dotenvy, AppState::new, router, worker spawn, serve
  src/config.rs        Arguments (clap, every field flag+env) → Configuration
                       DATABASE_URL, STORE_DIR (default ./store), COOKIE_KEY(+_FILE,
                       required ArgGroup), BIND_ADDR, --dev/VITE_URL/STATIC_DIR,
                       --anonymous (dev auth short-circuit), --create-admin user:pass
  src/state.rs         AppState { config, db: PgPool, store: BlobStore }
  src/extractors.rs    User extractor: PrivateCookieJar → users row; role checks;
                       anonymous short-circuit in dev
  src/routes/
    api.rs             /api/* (utoipa spec, swagger at /docs)
    auth.rs            /auth/register, /auth/login, /auth/logout
    frontend.rs        catch-all: static serve (SPA fallback) or Vite proxy + WS HMR
  src/services/
    blobstore.rs       trait BlobStore { put(stream)->sha256, get(sha)->stream, delete };
                       FsBlobStore: write to tmp, hash while streaming, rename into
                       store/ab/cd/<hash>; GET with range support
    jobs.rs            enqueue(); worker loop: FOR UPDATE SKIP LOCKED poll, retry
                       with backoff, per-kind dispatch
    importer.rs        import_archive job: unzip → hash → create blobs/files
                       preserving paths → enqueue render_preview per model file
    renderer.rs        render_preview job: read 'renderer' setting, shell out to
                       f3d (`f3d --output <png> <stl>` headless), store PNG blob,
                       insert images row stamped with renderer+config
  migrations/0001_initial.sql
  build.rs             APP_VERSION from git describe
frontend/              Vite + React + TS + MUI + react-router + @tanstack/react-query
docker-compose.yml     postgres:17 (+ volume); store/ is a bind-mounted dir
.env.example
```

## API surface (milestone 1)

- `POST /auth/register|login|logout`; `GET /api/me`
- `GET/POST/PUT/DELETE /api/creators`
- `GET/POST/PUT/DELETE /api/models` (+ `?tags=&q=&opt=axis:value&opt=…` unified
  search — `opt` is repeatable, one per axis, values resolved against the
  declarable axis/option tables)
- `GET/POST/PUT/DELETE /api/variant-axes` and `…/variant-axes/{id}/options` —
  manage declarable categories/options (editor+); variant create/update takes
  `{axis: option}` assignments, and the UI comboboxes offer "add new option"
  inline so new axes/options appear organically during import
- `GET/POST/PUT/DELETE /api/models/{id}/variants`
- `PUT /api/models/{id}/description` (creates a new revision);
  `GET /api/models/{id}/description/revisions` (history);
  `PUT …/revisions/{rev}/label` (name a revision, e.g. "v1")
- `POST /api/variants/{id}/files` — multipart upload; a `.zip` triggers an
  `import_archive` job (original archive kept as kind='archive'); others stored
  directly with an optional `path`
- `GET /api/files/{id}/download` (streams from blob store, Content-Disposition)
- `GET/POST /api/models/{id}/images` upload; `GET /api/images/{id}` (serve)
- `GET/POST /api/tags`; tag assignment on model create/update
- `GET /api/jobs?status=` (visibility into queue); `POST /api/jobs/{id}/retry`
- Admin: `GET/PUT /api/admin/settings/renderer`;
  `POST /api/admin/rerender { scope: "stale", mode: "add"|"replace" }` —
  enqueues render jobs for images whose renderer/config ≠ current setting
- Permissions: viewer = read + marks; editor = edit own models/bundles;
  admin = edit all + settings

## Frontend pages (MUI)

**Look & feel: vaguely Printables** (user request). MUI theme with Printables-style
orange primary (~`#FA6831`) on a clean white surface, plus a dark mode variant;
layout mirrors printables.com: top app bar (logo, big centred search, user menu),
model browsing as a dense card grid of large square-ish thumbnails with
name/creator/like-count underneath, and a left filter sidebar on the browse page
(tags, plus one filter group per declared variant axis, generated dynamically
from the axis/option tables).

- Login/Register
- Model grid: thumbnail cards, text search, tag chips, dynamic per-axis filters
- Model detail: image gallery (primary image), rendered-markdown description
  (edit dialog with revision history + "name this version"), variant list with
  per-variant file tree (rebuilt from `path`), download buttons, print notes,
  tags, creator link
- Model create/edit + upload/import dialog (drag-drop, zip import progress via
  job polling)
- Creators list/detail
- Admin settings page (renderer config + "re-render stale" button with add/replace)

## Implementation order

1. Scaffold backend+frontend per template; docker-compose postgres; `.env.example`;
   verify dev-mode proxy + HMR works end to end.
2. Migration 0001 (full schema above); sqlx offline metadata (`cargo sqlx prepare`).
3. Auth: argon2id, private cookie, User extractor, roles, `--create-admin`,
   `--anonymous` dev short-circuit.
4. `FsBlobStore` + upload/download routes (streaming, never buffering whole files
   in memory — bundles are multi-GB).
5. Creators/models/variants/tags CRUD + unified search query.
6. Job queue worker + `import_archive` job (zip extraction preserving structure).
7. Renderer setting + `render_preview` (f3d shell-out; graceful failure → job
   'failed' with error, model still browsable) + admin re-render endpoint.
8. Frontend pages in the order above.
9. `backend/CLAUDE.md` + top-level `CLAUDE.md` (run instructions), update
   `docs/spec.md` open questions as resolved (or a `docs/decisions.md`).

## Suggestions / gaps in the spec (to record in docs/decisions.md)

Schema above already accommodates these; flagged for the user:

- **License + purchase tracking** on models (price, date, order ref, commercial
  terms) — implied by "models I have bought" but not spec'd.
- **Keep the original archive** as a blob (kind='archive') for provenance/re-import.
- **Import is necessarily staged**: a Loot Studios zip contains many variants, so
  upload → background extract/hash → assignment of folders to variants (heuristic
  prefill from folder names like "32mm/Supported", user-correctable). Milestone 1
  imports a zip into ONE variant; multi-variant classification UI is a follow-up.
- **Model updates**: creators re-release files; `files.created_at` + re-import into
  the same variant covers v1, real versioning deferred.
- **Maintenance jobs**: orphan-blob GC and store integrity re-hash fit the job
  system; deferred but the design allows them.
- **"Printed" is richer than a flag**: printer/resin/exposure/outcome logs —
  `user_model_marks.notes` for now, a `print_logs` table later.
- **Browser import helper** needs token auth (not cookies) — follow-up.

## Verification

- `cargo test` (unit: blobstore hashing/rename, job claim semantics, search query
  building) + `cargo clippy`; `npm run build` clean.
- End-to-end via /verify: `docker compose up -d postgres`; run backend `--dev
  --anonymous` + Vite; then through the real UI/API: register/login (non-anonymous
  run), create creator + model + a variant assigned 32mm/unsupported from the
  seeded axes (and declare a new axis+option inline to prove extensibility),
  edit the markdown description twice and name a revision "v1", upload a zip,
  watch import job complete, confirm folder structure and file download
  round-trips byte-identical (hash check), tag it, search by text + tag + axis
  options (verifying same-variant AND semantics), upload an
  image; if `f3d` is installed locally, confirm a preview renders and lands in the
  gallery, then change renderer args and use re-render(replace) on the stale image.
- Duplicate check: upload the same STL twice → one blob on disk, two file rows.
