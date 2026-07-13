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
- **Tags vs variants (spec issue b)**: separate systems, unified at the search
  API (`?q=&tags=&opts=axis:value,…`). Variant attributes are declarable data
  (`variant_axes` + `variant_axis_options`), not enums — the spec's
  32mm/75mm/unsupported/… examples are seed rows only. Option filters require
  a single variant to satisfy all pairs.
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

## Deferred (schema already accommodates)

- Multi-variant zip classification UI (import currently targets one variant;
  folder-name heuristics like "32mm/Supported" prefill later).
- Bundles UI, likes/printed/wanted UI (`user_model_marks` exists).
- Orphan-blob GC + store integrity re-hash as maintenance jobs.
- Print logs richer than a mark (`print_logs` table later).
- Browser-import helper (needs token auth, not cookies).
- Duplicate-discovery report UI over shared `blob_sha256`.
