-- MeshTrove initial schema. See docs/plan.md for the ER diagram and rationale.

CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ---------------------------------------------------------------------------
-- auth
-- ---------------------------------------------------------------------------

CREATE TYPE user_role AS ENUM ('admin', 'editor', 'viewer');

CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    username      citext NOT NULL UNIQUE,
    password_hash text NOT NULL,
    role          user_role NOT NULL DEFAULT 'viewer',
    created_at    timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- provenance: the company / site / author a model came from
-- ---------------------------------------------------------------------------

CREATE TYPE creator_kind AS ENUM ('author', 'company', 'site');

CREATE TABLE creators (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name       text NOT NULL,
    kind       creator_kind NOT NULL DEFAULT 'author',
    url        text,
    notes      text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX creators_name_trgm ON creators USING gin (name gin_trgm_ops);

-- ---------------------------------------------------------------------------
-- core catalogue
-- ---------------------------------------------------------------------------

CREATE TABLE models (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name           text NOT NULL,
    slug           text NOT NULL UNIQUE,
    creator_id     uuid REFERENCES creators (id) ON DELETE SET NULL,
    source_url     text,
    license        text,
    purchase_price numeric(10, 2),
    purchase_date  date,
    order_ref      text,
    created_by     uuid NOT NULL REFERENCES users (id),
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    -- maintained by triggers below: name (A) + tags (B) + creator (B) +
    -- current description revision (C)
    search         tsvector NOT NULL DEFAULT ''::tsvector
);
CREATE INDEX models_search_idx ON models USING gin (search);
CREATE INDEX models_name_trgm ON models USING gin (name gin_trgm_ops);
CREATE INDEX models_creator_idx ON models (creator_id);

-- Markdown descriptions with full edit history; every save is a new immutable
-- revision (current = newest), optionally nameable ("v1", "v2").
CREATE TABLE model_description_revisions (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id   uuid NOT NULL REFERENCES models (id) ON DELETE CASCADE,
    body_md    text NOT NULL,
    label      citext,
    created_by uuid NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX model_description_labels
    ON model_description_revisions (model_id, label) WHERE label IS NOT NULL;
CREATE INDEX model_description_by_model
    ON model_description_revisions (model_id, created_at DESC);

CREATE TABLE model_variants (
    id                      uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id                uuid NOT NULL REFERENCES models (id) ON DELETE CASCADE,
    name                    text NOT NULL,
    derived_from_variant_id uuid REFERENCES model_variants (id) ON DELETE SET NULL,
    print_notes             text,
    created_by              uuid NOT NULL REFERENCES users (id),
    created_at              timestamptz NOT NULL DEFAULT now(),
    UNIQUE (model_id, name)
);

-- ---------------------------------------------------------------------------
-- variant attributes: declarable categories ("axes") with declarable options.
-- No hard-coded enums — scale/support below are only seed data.
-- ---------------------------------------------------------------------------

CREATE TABLE variant_axes (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        citext NOT NULL UNIQUE,
    description text,
    sort_order  int NOT NULL DEFAULT 0,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE variant_axis_options (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    axis_id    uuid NOT NULL REFERENCES variant_axes (id) ON DELETE CASCADE,
    value      citext NOT NULL,
    sort_order int NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (axis_id, value),
    UNIQUE (axis_id, id)  -- target for the composite FK from variant_options
);

CREATE TABLE variant_options (
    variant_id uuid NOT NULL REFERENCES model_variants (id) ON DELETE CASCADE,
    axis_id    uuid NOT NULL REFERENCES variant_axes (id) ON DELETE CASCADE,
    option_id  uuid NOT NULL,
    PRIMARY KEY (variant_id, axis_id),  -- one option per axis per variant
    -- composite FK guarantees the option actually belongs to the axis
    FOREIGN KEY (axis_id, option_id)
        REFERENCES variant_axis_options (axis_id, id) ON DELETE CASCADE
);
CREATE INDEX variant_options_by_option ON variant_options (option_id);

-- ---------------------------------------------------------------------------
-- content-addressed storage
-- ---------------------------------------------------------------------------

CREATE TABLE blobs (
    sha256     char(64) PRIMARY KEY,
    size       bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- bundles (purchasable packs AND personal uber-bundles), declared before
-- files/images which reference them
-- ---------------------------------------------------------------------------

CREATE TYPE bundle_kind AS ENUM ('purchased', 'collection');

CREATE TABLE bundles (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name       text NOT NULL,
    slug       text NOT NULL UNIQUE,
    creator_id uuid REFERENCES creators (id) ON DELETE SET NULL,
    source_url text,
    kind       bundle_kind NOT NULL DEFAULT 'purchased',
    created_by uuid NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE bundle_description_revisions (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id  uuid NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    body_md    text NOT NULL,
    label      citext,
    created_by uuid NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX bundle_description_labels
    ON bundle_description_revisions (bundle_id, label) WHERE label IS NOT NULL;
CREATE INDEX bundle_description_by_bundle
    ON bundle_description_revisions (bundle_id, created_at DESC);

CREATE TABLE bundle_models (
    bundle_id uuid NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    model_id  uuid NOT NULL REFERENCES models (id) ON DELETE CASCADE,
    PRIMARY KEY (bundle_id, model_id)
);

CREATE TABLE bundle_children (
    parent_bundle_id uuid NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    child_bundle_id  uuid NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    PRIMARY KEY (parent_bundle_id, child_bundle_id),
    CHECK (parent_bundle_id <> child_bundle_id)
);

-- ---------------------------------------------------------------------------
-- files: logical folder structure over the blob store.
-- variant files = printable parts; model/bundle files = associated documents
-- (stat guides, painting guides, magazines); kind='archive' keeps original
-- uploaded zips for provenance. Duplicate discovery = shared blob_sha256.
-- ---------------------------------------------------------------------------

CREATE TYPE file_kind AS ENUM ('model', 'document', 'archive', 'other');

CREATE TABLE files (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    blob_sha256 char(64) NOT NULL REFERENCES blobs (sha256),
    model_id    uuid REFERENCES models (id) ON DELETE CASCADE,
    variant_id  uuid REFERENCES model_variants (id) ON DELETE CASCADE,
    bundle_id   uuid REFERENCES bundles (id) ON DELETE CASCADE,
    path        text NOT NULL DEFAULT '',
    filename    text NOT NULL,
    mime        text,
    kind        file_kind NOT NULL DEFAULT 'other',
    created_at  timestamptz NOT NULL DEFAULT now(),
    CHECK (num_nonnulls(model_id, variant_id, bundle_id) = 1)
);
CREATE INDEX files_by_blob ON files (blob_sha256);
CREATE INDEX files_by_variant ON files (variant_id) WHERE variant_id IS NOT NULL;
CREATE INDEX files_by_model ON files (model_id) WHERE model_id IS NOT NULL;
CREATE INDEX files_by_bundle ON files (bundle_id) WHERE bundle_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- images: attach to models, variants, or bundles; rendered ones record the
-- renderer + config that produced them so stale ones can be re-rendered
-- ---------------------------------------------------------------------------

CREATE TYPE image_kind AS ENUM ('uploaded', 'imported', 'rendered');

CREATE TABLE images (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    blob_sha256     char(64) NOT NULL REFERENCES blobs (sha256),
    model_id        uuid REFERENCES models (id) ON DELETE CASCADE,
    variant_id      uuid REFERENCES model_variants (id) ON DELETE CASCADE,
    bundle_id       uuid REFERENCES bundles (id) ON DELETE CASCADE,
    kind            image_kind NOT NULL DEFAULT 'uploaded',
    source_file_id  uuid REFERENCES files (id) ON DELETE SET NULL,
    renderer        text,
    renderer_config jsonb,
    mime            text,
    width           int,
    height          int,
    is_primary      boolean NOT NULL DEFAULT false,
    sort_order      int NOT NULL DEFAULT 0,
    created_by      uuid REFERENCES users (id),
    created_at      timestamptz NOT NULL DEFAULT now(),
    CHECK (num_nonnulls(model_id, variant_id, bundle_id) = 1)
);
-- The "Primary" preview image: at most one per owner
CREATE UNIQUE INDEX images_primary_per_model ON images (model_id)
    WHERE is_primary AND model_id IS NOT NULL;
CREATE UNIQUE INDEX images_primary_per_variant ON images (variant_id)
    WHERE is_primary AND variant_id IS NOT NULL;
CREATE UNIQUE INDEX images_primary_per_bundle ON images (bundle_id)
    WHERE is_primary AND bundle_id IS NOT NULL;
CREATE INDEX images_by_model ON images (model_id) WHERE model_id IS NOT NULL;
CREATE INDEX images_by_variant ON images (variant_id) WHERE variant_id IS NOT NULL;
CREATE INDEX images_by_bundle ON images (bundle_id) WHERE bundle_id IS NOT NULL;
CREATE INDEX images_by_blob ON images (blob_sha256);

-- ---------------------------------------------------------------------------
-- tagging
-- ---------------------------------------------------------------------------

CREATE TABLE tags (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name       citext NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX tags_name_trgm ON tags USING gin (name gin_trgm_ops);

CREATE TABLE model_tags (
    model_id uuid NOT NULL REFERENCES models (id) ON DELETE CASCADE,
    tag_id   uuid NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (model_id, tag_id)
);
CREATE INDEX model_tags_by_tag ON model_tags (tag_id);

CREATE TABLE bundle_tags (
    bundle_id uuid NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    tag_id    uuid NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (bundle_id, tag_id)
);
CREATE INDEX bundle_tags_by_tag ON bundle_tags (tag_id);

-- ---------------------------------------------------------------------------
-- per-user marks (liked / printed / wanted)
-- ---------------------------------------------------------------------------

CREATE TYPE mark_kind AS ENUM ('liked', 'printed', 'wanted');

CREATE TABLE user_model_marks (
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    model_id   uuid NOT NULL REFERENCES models (id) ON DELETE CASCADE,
    mark       mark_kind NOT NULL,
    notes      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, model_id, mark)
);
CREATE INDEX marks_by_model ON user_model_marks (model_id, mark);

-- ---------------------------------------------------------------------------
-- background jobs (imports, preview renders, future GC/verify)
-- ---------------------------------------------------------------------------

CREATE TYPE job_status AS ENUM ('queued', 'running', 'succeeded', 'failed', 'cancelled');

CREATE TABLE jobs (
    id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    kind         text NOT NULL,
    payload      jsonb NOT NULL DEFAULT '{}',
    status       job_status NOT NULL DEFAULT 'queued',
    priority     int NOT NULL DEFAULT 0,
    attempts     int NOT NULL DEFAULT 0,
    max_attempts int NOT NULL DEFAULT 3,
    last_error   text,
    run_after    timestamptz NOT NULL DEFAULT now(),
    started_at   timestamptz,
    finished_at  timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX jobs_claim_idx ON jobs (status, priority DESC, run_after);

-- ---------------------------------------------------------------------------
-- admin-global settings, e.g. key 'renderer' → {"tool": "f3d", "args": [...]}
-- ---------------------------------------------------------------------------

CREATE TABLE settings (
    key        text PRIMARY KEY,
    value      jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid REFERENCES users (id)
);

-- ---------------------------------------------------------------------------
-- search vector maintenance
-- ---------------------------------------------------------------------------

CREATE FUNCTION model_search_vector(model_id uuid) RETURNS tsvector AS $$
    SELECT setweight(to_tsvector('english', m.name), 'A')
         || setweight(to_tsvector('english', coalesce(c.name, '')), 'B')
         || setweight(to_tsvector('english', coalesce(
                (SELECT string_agg(t.name::text, ' ')
                   FROM model_tags mt JOIN tags t ON t.id = mt.tag_id
                  WHERE mt.model_id = m.id), '')), 'B')
         || setweight(to_tsvector('english', coalesce(
                (SELECT r.body_md FROM model_description_revisions r
                  WHERE r.model_id = m.id
                  ORDER BY r.created_at DESC LIMIT 1), '')), 'C')
      FROM models m LEFT JOIN creators c ON c.id = m.creator_id
     WHERE m.id = model_id
$$ LANGUAGE sql STABLE;

CREATE FUNCTION refresh_model_search() RETURNS trigger AS $$
BEGIN
    UPDATE models SET search = model_search_vector(id) WHERE id = NEW.id;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

-- On the models row itself: compute inline (BEFORE) to avoid recursion
CREATE FUNCTION models_search_before() RETURNS trigger AS $$
BEGIN
    NEW.search := setweight(to_tsvector('english', NEW.name), 'A')
        || setweight(to_tsvector('english', coalesce(
               (SELECT c.name FROM creators c WHERE c.id = NEW.creator_id), '')), 'B')
        || setweight(to_tsvector('english', coalesce(
               (SELECT string_agg(t.name::text, ' ')
                  FROM model_tags mt JOIN tags t ON t.id = mt.tag_id
                 WHERE mt.model_id = NEW.id), '')), 'B')
        || setweight(to_tsvector('english', coalesce(
               (SELECT r.body_md FROM model_description_revisions r
                 WHERE r.model_id = NEW.id
                 ORDER BY r.created_at DESC LIMIT 1), '')), 'C');
    RETURN NEW;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER models_search_refresh
    BEFORE INSERT OR UPDATE OF name, creator_id ON models
    FOR EACH ROW EXECUTE FUNCTION models_search_before();

CREATE FUNCTION refresh_model_search_from_child() RETURNS trigger AS $$
DECLARE
    target uuid;
BEGIN
    target := coalesce(NEW.model_id, OLD.model_id);
    UPDATE models SET search = model_search_vector(id) WHERE id = target;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER model_tags_search_refresh
    AFTER INSERT OR DELETE ON model_tags
    FOR EACH ROW EXECUTE FUNCTION refresh_model_search_from_child();

CREATE TRIGGER model_description_search_refresh
    AFTER INSERT ON model_description_revisions
    FOR EACH ROW EXECUTE FUNCTION refresh_model_search_from_child();

-- Renaming a tag or creator re-vectorizes affected models
CREATE FUNCTION refresh_models_for_tag() RETURNS trigger AS $$
BEGIN
    UPDATE models SET search = model_search_vector(id)
     WHERE id IN (SELECT model_id FROM model_tags WHERE tag_id = NEW.id);
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER tags_rename_refresh
    AFTER UPDATE OF name ON tags
    FOR EACH ROW EXECUTE FUNCTION refresh_models_for_tag();

CREATE FUNCTION refresh_models_for_creator() RETURNS trigger AS $$
BEGIN
    UPDATE models SET search = model_search_vector(id) WHERE creator_id = NEW.id;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER creators_rename_refresh
    AFTER UPDATE OF name ON creators
    FOR EACH ROW EXECUTE FUNCTION refresh_models_for_creator();

-- ---------------------------------------------------------------------------
-- seed data: the spec's variant examples, as editable rows (NOT a fixed list)
-- ---------------------------------------------------------------------------

WITH scale AS (
    INSERT INTO variant_axes (name, description, sort_order)
    VALUES ('scale', 'Physical scale of the model, e.g. 32mm, 75mm', 0)
    RETURNING id
)
INSERT INTO variant_axis_options (axis_id, value, sort_order)
SELECT id, v.value, v.ord FROM scale,
    (VALUES ('32mm', 0), ('75mm', 1)) AS v (value, ord);

WITH support AS (
    INSERT INTO variant_axes (name, description, sort_order)
    VALUES ('support', 'Support/preparation state of the files', 1)
    RETURNING id
)
INSERT INTO variant_axis_options (axis_id, value, sort_order)
SELECT id, v.value, v.ord FROM support,
    (VALUES ('unsupported', 0), ('supported', 1), ('supported_hollow', 2),
            ('lychee_project', 3), ('merged', 4)) AS v (value, ord);
