# Design decisions

Resolutions of the open questions in `spec.md`, plus gaps found during design.
Full schema/architecture: `plan.md`.

## Status (2026-07-13)

**Milestone 1 ("core archive") is complete and verified end-to-end.**
Everything in `plan.md` is implemented except the items under "Deferred"
below. Verified live, not just compiled: zip upload → import job (folder
structure kept) → f3d preview render → primary image on the browse card;
unified search including the same-variant AND semantics for axis options;
dedup (same STL twice → one blob, two file rows, byte-identical download);
register/login/logout with role assignment; stale re-render converging to
zero queued jobs. UI screenshots reviewed in headless Chromium with no
console errors. A fresh session should pick up from the Deferred list.

Implementation quirks worth knowing (found the hard way):

- MUI v9 dropped direct system props on `Stack`/`Toolbar` — put
  `alignItems` etc. in `sx`, or tsc fails with opaque overload errors.
- `url::Url` Display always renders a trailing `/`; the Vite proxy must
  trim it or `/@vite/client` becomes `//@vite/client` and modules come
  back as `text/html` (frontend.rs has the fix + comment).
- f3d obeys the user's `~/.config/f3d` — renders must pass `--no-config`
  or grid/axis/filename overlays leak into previews.
- Replace-mode re-renders delete the old image *before* inserting, in the
  same transaction, so the primary slot carries over (renderer.rs).

## Resolved

- **Name**: MeshTrove (crate `meshtrove`, env prefix `MESHTROVE_`).
- **Storage (spec issue a)**: content-addressed filesystem blob store
  (`store/ab/cd/<sha256>`) behind a `BlobStore` trait; Postgres owns all
  metadata including folder structure. Dedup is inherent (same hash = same
  blob, verified: same STL uploaded twice → 1 blob, 2 file rows). S3 was
  rejected for now: a second stateful service with no payoff at single-box
  scale; the trait keeps the door open.
- **Tags vs variants (spec issue b)**: separate systems with separate
  vocabularies, unified at the search API (`?q=&tags=&vtags=…`). `tags` say what
  a model *is* (dragon, terrain); `variant_tags` say which *edition* of it a file
  belongs to (32mm, supported). Keeping them apart stops "supported" turning up
  in the subject tag cloud. Variant tag filters require a **single variant** to
  carry all of them, so 32mm + unsupported does not match a model whose 32mm and
  unsupported tags sit on different variants.
- **A variant IS its tag set**: `model_variants.name` is an optional display
  label; identity is the set of variant tags, canonicalised into `tag_key` by
  trigger and made unique per model. Three things follow, and all three were the
  point:
  - **Tags are flat, not axis/value.** A variant can carry two tags that would
    have shared an axis (an `stl` *and* `obj` edition). `variant_axes` /
    `variant_axis_options` / `variant_options` are gone; their option values
    survive as flat tags.
  - **An anonymous variant is legal.** No name, no tags: the plain bucket of
    files a model separates out without asserting a tag for them. `UNIQUE
    (model_id, tag_key)` allows exactly one per model, since its key is `''`.
  - **Collisions merge, they never conflict.** Creating or retagging a variant
    onto a tag set the model already has returns *that* variant and folds the
    files and images across (`merge_duplicate_variants()`, plus the app-level
    path in `routes/variants.rs`). Two variants with the same tags were never
    two variants. Duplicate *names* are still rejected — that is a mistake, not
    a merge.
- **Rendering**: external tool via background job, f3d first. Admin-global
  `renderer` setting ({input}/{output} substitution); each rendered image
  records renderer+config, so "re-render stale" (add or replace) is a bulk
  admin action. Default args include `--no-config` so user f3d configs can't
  leak overlays into previews.
- **Descriptions**: markdown, stored as immutable revisions with optional
  unique labels ("v1"); models and bundles get identical treatment.
- **Primary image**: at most one per model/variant/bundle via partial unique
  indexes; first upload auto-primary; atomic swap endpoint.

## Additions beyond the spec

- License + purchase tracking (price/date/order ref) on models.
- Original uploaded archives are **not** kept (reversed 2026-07-14, migration
  0006). Keeping the zip as a `kind='archive'` file row meant the store held the
  archive *and* every file unpacked out of it — a permanent ~1.3–1.5x surcharge
  for bytes nobody browses, and the direct cause of a disk filling mid-upload.
  Committing an import now deletes the archive and writes a `source_archives`
  row: filename, sha256, size — the provenance anyone actually asks for ("what
  was this dropped from?"), at ~100 bytes instead of gigabytes. An import that is
  *only* an archive (its unpack failed) keeps it: there would be nothing left.
- **Orphan-blob GC** (`services/gc.rs`) exists because of the above. Blobs are
  shared, so deleting a `files` row can never delete bytes on its own; `collect_blob`
  drops a blob only when no `files` *or* `images` row still points at it, and only
  after the transaction that removed the last reference has committed.
- First registered user becomes admin; later users start as viewers.
- **The dropbox** (`<store>/imports`, added 2026-07-18): a folder an admin fills
  server-side — over ssh, a file share, a torrent client's completed dir — and
  stages from the Importing page with one button. The browser is the wrong pipe
  for bytes already on the machine: uploading a 40GB box set that is sitting next
  to the store copies it back over the network to land where it started. A pickup
  reads it in place. It is deliberately *not* a second import mechanism — the
  pickup creates the same staged import a drop does, roots a folder's paths at
  its own name exactly as a browser folder-drop does, and hands any zip to the
  same `import_archive` job (`routes/files::on_archive_ingested` is shared by
  both doors). The copy runs as a `dropbox_import` job, not inline, because the
  entry can be tens of gigabytes; `ImportSummary.unpacking` covers that job too,
  so the existing "still unpacking, can't commit" guard covers a pickup for free.
  Admin-only, unlike the rest of the import routes, because it reads the server's
  filesystem. Picking up never modifies the dropbox — the entry stays until the
  admin deletes it, so a failed import is always retryable.
- **Job workers are concurrent and lane-split, not a separate process** (added
  2026-07-21). `--job-workers` (default 1) and `--render-workers` (default 2)
  spawn N worker tasks each; the claim query already used `FOR UPDATE SKIP
  LOCKED`, so concurrency needed no locking work. Archive work defaults to a
  single worker: unpacking is disk-bound, so a second concurrent import buys
  little and competes for the same spindle. The *lanes* are the point: a
  render is short and the UI is waiting on it, so it must not queue behind a
  40GB dropbox import that happens to have a lower id. `Lane::Render` claims an
  allowlist of kinds and `Lane::General` claims its complement, so the two
  always partition every kind — a kind added to `dispatch` and forgotten here
  still runs rather than sitting queued forever with no worker willing to take
  it. They stay *in* the server binary because `recover_stranded` requeues every
  `running` row at startup, which is only safe while one process owns them all;
  a second worker process would yank jobs out from under the first, and would
  need leases/heartbeats first. Pool sizing moved with it — a worker holds its
  connection for the whole job, so `max_connections` is now `10 + workers`
  rather than a flat 10 shared with every browse query.

## File-first import + recategorisation (Phases 1–2 done, 3 remaining)

Phased build of drag-drop import + a classification UI (was under *Deferred*).
Real archive shapes driving it: `docs/import-layouts.md`.

- **Phase 1 (done):** drop a file anywhere (global overlay) or into the New
  Model dialog → *(a model was auto-created here; a drop now stages an import
  instead — see below)* → a `.zip` unpacks **flat into the model's "unsorted"
  bucket** (model-owned
  `files`, `variant_id` null). The model page's *Unsorted files* section
  reclassifies file kinds, moves a selection into variants (new — with
  scale/support axes — or existing), and deletes files. New API:
  `GET /api/models/{id}/files`, `PATCH`/`DELETE /api/files/{id}`; the importer
  now accepts model-owned archives and the `.zip` trigger fires for model
  uploads too. No schema migration (the `files.model_id` nullable owner + the
  `num_nonnulls(...) = 1` CHECK already allowed this).
- **Phase 2 (done):** bundles. Migration 0002 adds a `search` tsvector to
  `bundles` (mirrors models) so they rank **inline** with models in a unified
  `GET /api/browse` (UNION ALL, ranked+paginated together; a variant-axis `opt=`
  filter excludes bundles). `routes/bundles.rs` = CRUD + description revisions +
  `bundle_models` membership. UI: `BundleCard`/`BundlePage`/`BundleEditDialog`,
  mixed browse grid. ("Promote to bundle" lived here; imports removed the need
  for it.) **Drop onto a bundle** stages an import preset to that bundle, whose
  files land in the bundle's own unsorted bucket (`files.bundle_id`); the bundle
  page carves those files into new/existing **member models**
  (`PATCH /api/files/{id}` with `model_id`, validated against `bundle_models`) —
  symmetric with Phase 1 (model→variants), one level up (bundle→models→variants).
  Deferred within bundles: `bundle_children` nesting; bundle likes.
- **Imports — the staging area (done, supersedes part of Phases 1–2):** models
  and bundles are now *fixed* kinds; nothing converts into anything. A dropped
  archive lands in an **import** (migration 0003: `imports` table +
  `files.import_id`, the files CHECK widened to four owners), which is neither a
  model nor a bundle and never appears in browse/search — only on the *Importing*
  list. Once the unpack job finishes you see the real contents and pick one
  destination: **new model**, **new bundle**, or **add to an existing bundle**
  (preselected when the drop happened on a bundle page).
  `POST /api/imports/{id}/commit` moves every staged file onto that owner in one
  transaction and drops the import row; it refuses while an unpack is in flight,
  so files can't be stranded.

  **Folders drop too.** A dropped directory arrives in `dataTransfer.files`
  disguised as a file — a name, a size — and only fails when something reads its
  bytes (Firefox `NS_ERROR_FILE_IS_DIRECTORY`; Chrome/Safari silently upload zero
  bytes). `webkitGetAsEntry()` is the only way to tell, and only inside the drop
  handler, so `readDrop` (`frontend/src/upload.ts`) walks the tree there and
  stages each file with its folder in `path` — an unzipped folder imports
  identically to the same folder zipped. No backend change: the multipart
  contract already applies a `path` field to every `file` part that follows it,
  so one `path`/`file` pair per file carries the whole tree in one request.

  *Why:* the model-vs-bundle question can't be answered at drop time — the
  contents aren't known yet, and the archive filename doesn't say. Deferring it
  by one step removes the guess entirely, and with it the need for "Promote to
  bundle" / "Flatten to model" conversions (both now gone). A dedicated table,
  rather than a `status` flag on `bundles`, means a staged import cannot leak
  into the library through a query that forgot to filter it out.
- **Phase 3 (carve done 2026-07-14, heuristics partial) — suggestions, once the
  contents are known:** Phase 3 reads the **staged file tree** — which imports
  made available *before* anything is committed — and turns each choice on the
  import page into a pre-filled default:
  - **Destination (partial):** the layout panel's multi-model-name hint covers
    the bundle direction ("this layout finds N model names — make it a
    bundle"); a folder-shape heuristic preselecting the toggle before any
    layout is chosen remains todo.
  - **Name (done via layouts):** a one-model layout with a single captured
    model name prefills the name field, unless the user typed their own.
  - **The carve (done 2026-07-14):** regex **import layout templates** with
    role-assigned capture groups — full design under *Import layout templates*
    in `docs/plan.md`. Implemented as migration 0005 (`import_layouts` +
    seeded Loot Studios preset), `services/layout.rs` (`analyze`, fancy-regex,
    unit-tested), `POST /api/imports/{id}/plan` (dry run: coverage, grouped
    tree, per-file highlight spans + resolved chips), `layout` on every commit
    target (atomic carve; reuses matching member models so a 75mm drop lands
    on the 32mm drop's models; refuses unmapped values), `/api/import-layouts`
    CRUD, and the `ImportLayoutPanel` + annotated file list on `ImportPage`.

  *Why it stayed a suggestion:* imports removed the need to guess at drop time,
  so a detector that is wrong now costs an edit, not a migration between kinds.
  That is the only reason it is safe to add heuristics at all.

## Deferred (schema already accommodates)

- **Tests beyond the 17 unit tests** — backend HTTP tests (`#[sqlx::test]` +
  `oneshot`), then Vitest/RTL, then Playwright, then CI. Order and rationale in
  `docs/plan.md`, "Test strategy": every bug that escaped so far was a backend
  *route* bug, which is the layer with no tests at all.
- Likes/printed/wanted UI (`user_model_marks` exists) — note nothing can *set* a
  like today (no endpoint, no button), so every `like_count` is 0; the card hides
  the heart until it is non-zero. The anonymous dev user is `Uuid::nil()` with no
  `users` row, which the `user_model_marks` FK would reject.
- Orphan-blob GC + store integrity re-hash as maintenance jobs.
- Print logs richer than a mark (`print_logs` table later).
- Browser-import helper (needs token auth, not cookies).
- Duplicate-discovery report UI over shared `blob_sha256`.
