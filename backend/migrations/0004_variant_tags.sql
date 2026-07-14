-- Variants are identified by their SET of tags, not by a name.
--
-- Two consequences drive this migration. First, the axis/option split is gone:
-- variant tags are a flat vocabulary, so one variant can carry two tags that
-- used to belong to the same axis (an "stl" AND "obj" edition). Second, a
-- variant may be ANONYMOUS — no name and no tags — which is how a model
-- separates a plain bucket of files out without asserting a variant tag for it.
--
-- Identity lives in `tag_key`, a canonical rendering of the tag set maintained
-- by trigger. UNIQUE (model_id, tag_key) then gives, for free, both "no two
-- variants of a model share a tag set" and "at most one anonymous variant per
-- model" (the anonymous one being the row whose key is the empty string).

CREATE TABLE variant_tags (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        citext NOT NULL UNIQUE,
    description text,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX variant_tags_name_trgm ON variant_tags USING gin (name gin_trgm_ops);

CREATE TABLE variant_tag_assignments (
    variant_id uuid NOT NULL REFERENCES model_variants (id) ON DELETE CASCADE,
    tag_id     uuid NOT NULL REFERENCES variant_tags (id) ON DELETE CASCADE,
    PRIMARY KEY (variant_id, tag_id)
);
CREATE INDEX variant_tag_assignments_by_tag ON variant_tag_assignments (tag_id);

-- The canonical key for a tag set. Sorting by uuid makes it order-insensitive,
-- and keying on ids (not names) keeps it stable across a tag rename. The empty
-- set collapses to '' — the anonymous variant.
CREATE FUNCTION variant_tag_key(tag_ids uuid[]) RETURNS text AS $$
    SELECT coalesce(string_agg(t::text, ',' ORDER BY t), '') FROM unnest(tag_ids) AS t
$$ LANGUAGE sql IMMUTABLE;

ALTER TABLE model_variants ALTER COLUMN name DROP NOT NULL;
ALTER TABLE model_variants DROP CONSTRAINT model_variants_model_id_name_key;
ALTER TABLE model_variants ADD COLUMN tag_key text NOT NULL DEFAULT '';

-- A name is now an optional display label, but two variants of a model sharing
-- one is still a mistake worth rejecting.
CREATE UNIQUE INDEX model_variants_name
    ON model_variants (model_id, name) WHERE name IS NOT NULL;

CREATE FUNCTION refresh_variant_tag_key() RETURNS trigger AS $$
BEGIN
    UPDATE model_variants SET tag_key = variant_tag_key(
        array(SELECT tag_id FROM variant_tag_assignments
               WHERE variant_id = coalesce(NEW.variant_id, OLD.variant_id)))
     WHERE id = coalesce(NEW.variant_id, OLD.variant_id);
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER variant_tag_assignments_key
    AFTER INSERT OR DELETE ON variant_tag_assignments
    FOR EACH ROW EXECUTE FUNCTION refresh_variant_tag_key();

-- Same tag set = same variant, so a collision is a merge, never an error: the
-- oldest row survives and absorbs the others' files and images. Callers that
-- can create a collision (deleting a tag, retagging a variant) run this after.
-- The survivor keeps its own primary image, so absorbed ones are demoted to
-- satisfy images_primary_per_variant.
CREATE FUNCTION merge_duplicate_variants() RETURNS void AS $$
DECLARE
    m record;
BEGIN
    FOR m IN
        SELECT id, keep FROM (
            SELECT id, first_value(id) OVER (
                       PARTITION BY model_id, tag_key ORDER BY created_at, id) AS keep
              FROM model_variants) d
         WHERE id <> keep
    LOOP
        UPDATE files  SET variant_id = m.keep WHERE variant_id = m.id;
        UPDATE images SET variant_id = m.keep, is_primary = false WHERE variant_id = m.id;
        UPDATE model_variants SET derived_from_variant_id = m.keep
         WHERE derived_from_variant_id = m.id;
        DELETE FROM model_variants WHERE id = m.id;
    END LOOP;
END
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------------
-- carry the axis/option vocabulary over: every declared option value becomes a
-- flat tag, and every assignment becomes a tag assignment. Distinct axes that
-- happen to share an option value ('merged' under two categories, say) collapse
-- to one tag — which is the point of going flat.
-- ---------------------------------------------------------------------------

INSERT INTO variant_tags (name)
SELECT DISTINCT value FROM variant_axis_options
ON CONFLICT (name) DO NOTHING;

INSERT INTO variant_tag_assignments (variant_id, tag_id)
SELECT vo.variant_id, vt.id
  FROM variant_options vo
  JOIN variant_axis_options o ON o.axis_id = vo.axis_id AND o.id = vo.option_id
  JOIN variant_tags vt ON vt.name = o.value
ON CONFLICT DO NOTHING;

DROP TABLE variant_options;
DROP TABLE variant_axis_options;
DROP TABLE variant_axes;

-- Variants that differed only by name now have identical tag sets. Under the
-- new rule they were always the same variant, so collapse them before the
-- constraint that says so goes on.
SELECT merge_duplicate_variants();

ALTER TABLE model_variants ADD CONSTRAINT model_variants_tag_set
    UNIQUE (model_id, tag_key) DEFERRABLE INITIALLY DEFERRED;
