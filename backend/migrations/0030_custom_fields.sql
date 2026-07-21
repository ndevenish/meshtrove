-- Custom fields: an admin-defined vocabulary of extra metadata that models and
-- bundles can carry. The set of fields is a meshtrove-wide setting, not a
-- per-entity one — an admin defines "Printed?" once and it is available on
-- every model (and/or every bundle) from then on.
--
-- Deliberately data-driven, in the same spirit as `variant_tags`: no hard-coded
-- column per user idea. A field says what it *is* (kind + options), where it
-- applies, who may see it, and whether a bundle's value flows down to its
-- member models.

CREATE TYPE custom_field_kind       AS ENUM ('text', 'checkbox', 'choice', 'rating', 'file');
CREATE TYPE custom_field_visibility AS ENUM ('anonymous', 'viewer', 'editor', 'admin');

CREATE TABLE custom_fields (
    id      uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Stable slug: what scraped metadata keys are matched against, so renaming
    -- the display label never breaks an ingest.
    key     citext NOT NULL UNIQUE,
    name    text   NOT NULL,
    kind    custom_field_kind NOT NULL,
    -- Kind-specific shape: choice → {"choices": [...]}, rating → {"max": 5}.
    options jsonb NOT NULL DEFAULT '{}',
    applies_to_models  boolean NOT NULL DEFAULT false,
    applies_to_bundles boolean NOT NULL DEFAULT false,
    -- When the bundle's value is written, copy it down to every member model...
    bundle_persists_to_model  boolean NOT NULL DEFAULT false,
    -- ...clobbering a value the model already had, or leaving it alone.
    bundle_persist_overwrites boolean NOT NULL DEFAULT false,
    visibility custom_field_visibility NOT NULL DEFAULT 'anonymous',
    position   integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    -- A field that applies to nothing is a field that does nothing.
    CONSTRAINT custom_fields_applies_somewhere CHECK (applies_to_models OR applies_to_bundles)
);

CREATE TABLE custom_field_values (
    id       uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    field_id uuid NOT NULL REFERENCES custom_fields (id) ON DELETE CASCADE,
    model_id  uuid REFERENCES models (id) ON DELETE CASCADE,
    bundle_id uuid REFERENCES bundles (id) ON DELETE CASCADE,
    -- text/checkbox/choice/rating live here; a file-kind value keeps this null
    -- and carries its payload as a `files` row pointing back at this row.
    value      jsonb,
    updated_by uuid REFERENCES users (id),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT cfv_one_owner CHECK (num_nonnulls(model_id, bundle_id) = 1)
);
CREATE UNIQUE INDEX cfv_model_uq  ON custom_field_values (field_id, model_id)  WHERE model_id  IS NOT NULL;
CREATE UNIQUE INDEX cfv_bundle_uq ON custom_field_values (field_id, bundle_id) WHERE bundle_id IS NOT NULL;
CREATE INDEX cfv_by_model  ON custom_field_values (model_id)  WHERE model_id  IS NOT NULL;
CREATE INDEX cfv_by_bundle ON custom_field_values (bundle_id) WHERE bundle_id IS NOT NULL;

-- A file-kind value owns its blob through a real `files` row — so download,
-- dedup and GC all work unchanged — but because the owner is the *value* and
-- not model_id/variant_id/bundle_id, it never shows up in a model's or bundle's
-- file list. Same move migration 0003 made when it added `import_id`.
ALTER TABLE files ADD COLUMN custom_field_value_id uuid
    REFERENCES custom_field_values (id) ON DELETE CASCADE;
ALTER TABLE files DROP CONSTRAINT files_one_owner;
ALTER TABLE files ADD CONSTRAINT files_one_owner
    CHECK (num_nonnulls(model_id, variant_id, bundle_id, import_id, custom_field_value_id) = 1);
-- One file per value: replacing the file replaces the row.
CREATE UNIQUE INDEX files_cfv_uq ON files (custom_field_value_id)
    WHERE custom_field_value_id IS NOT NULL;
