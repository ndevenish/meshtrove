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
- Original uploaded archives kept as `kind='archive'` file rows for provenance.
- First registered user becomes admin; later users start as viewers.

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

- Likes/printed/wanted UI (`user_model_marks` exists).
- Orphan-blob GC + store integrity re-hash as maintenance jobs.
- Print logs richer than a mark (`print_logs` table later).
- Browser-import helper (needs token auth, not cookies).
- Duplicate-discovery report UI over shared `blob_sha256`.
