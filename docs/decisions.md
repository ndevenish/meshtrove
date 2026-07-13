# Design decisions

Resolutions of the open questions in `spec.md`, plus gaps found during design.
Full schema/architecture: `plan.md`.

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
