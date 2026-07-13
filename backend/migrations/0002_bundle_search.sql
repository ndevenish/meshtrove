-- Bundle full-text search, mirroring the models machinery in 0001 so bundles
-- rank inline with models in the unified browse. Weights: name (A), creator (B),
-- tags (B), current description revision (C). Bundles have no variants, so there
-- is no opts/variant term.

ALTER TABLE bundles ADD COLUMN search tsvector NOT NULL DEFAULT ''::tsvector;

CREATE FUNCTION bundle_search_vector(bundle_id uuid) RETURNS tsvector AS $$
    SELECT setweight(to_tsvector('english', b.name), 'A')
         || setweight(to_tsvector('english', coalesce(c.name, '')), 'B')
         || setweight(to_tsvector('english', coalesce(
                (SELECT string_agg(t.name::text, ' ')
                   FROM bundle_tags bt JOIN tags t ON t.id = bt.tag_id
                  WHERE bt.bundle_id = b.id), '')), 'B')
         || setweight(to_tsvector('english', coalesce(
                (SELECT r.body_md FROM bundle_description_revisions r
                  WHERE r.bundle_id = b.id
                  ORDER BY r.created_at DESC LIMIT 1), '')), 'C')
      FROM bundles b LEFT JOIN creators c ON c.id = b.creator_id
     WHERE b.id = bundle_id
$$ LANGUAGE sql STABLE;

-- On the bundles row itself: compute inline (BEFORE) to avoid recursion
CREATE FUNCTION bundles_search_before() RETURNS trigger AS $$
BEGIN
    NEW.search := setweight(to_tsvector('english', NEW.name), 'A')
        || setweight(to_tsvector('english', coalesce(
               (SELECT c.name FROM creators c WHERE c.id = NEW.creator_id), '')), 'B')
        || setweight(to_tsvector('english', coalesce(
               (SELECT string_agg(t.name::text, ' ')
                  FROM bundle_tags bt JOIN tags t ON t.id = bt.tag_id
                 WHERE bt.bundle_id = NEW.id), '')), 'B')
        || setweight(to_tsvector('english', coalesce(
               (SELECT r.body_md FROM bundle_description_revisions r
                 WHERE r.bundle_id = NEW.id
                 ORDER BY r.created_at DESC LIMIT 1), '')), 'C');
    RETURN NEW;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER bundles_search_refresh
    BEFORE INSERT OR UPDATE OF name, creator_id ON bundles
    FOR EACH ROW EXECUTE FUNCTION bundles_search_before();

CREATE FUNCTION refresh_bundle_search_from_child() RETURNS trigger AS $$
DECLARE
    target uuid;
BEGIN
    target := coalesce(NEW.bundle_id, OLD.bundle_id);
    UPDATE bundles SET search = bundle_search_vector(id) WHERE id = target;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER bundle_tags_search_refresh
    AFTER INSERT OR DELETE ON bundle_tags
    FOR EACH ROW EXECUTE FUNCTION refresh_bundle_search_from_child();

CREATE TRIGGER bundle_description_search_refresh
    AFTER INSERT ON bundle_description_revisions
    FOR EACH ROW EXECUTE FUNCTION refresh_bundle_search_from_child();

-- Renaming a tag or creator re-vectorizes affected bundles (additive: the 0001
-- triggers on the same tables handle models; both fire).
CREATE FUNCTION refresh_bundles_for_tag() RETURNS trigger AS $$
BEGIN
    UPDATE bundles SET search = bundle_search_vector(id)
     WHERE id IN (SELECT bundle_id FROM bundle_tags WHERE tag_id = NEW.id);
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER bundle_tags_rename_refresh
    AFTER UPDATE OF name ON tags
    FOR EACH ROW EXECUTE FUNCTION refresh_bundles_for_tag();

CREATE FUNCTION refresh_bundles_for_creator() RETURNS trigger AS $$
BEGIN
    UPDATE bundles SET search = bundle_search_vector(id) WHERE creator_id = NEW.id;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER bundle_creators_rename_refresh
    AFTER UPDATE OF name ON creators
    FOR EACH ROW EXECUTE FUNCTION refresh_bundles_for_creator();

-- Backfill any existing rows (functions now exist).
UPDATE bundles SET search = bundle_search_vector(id);

CREATE INDEX bundles_search_idx ON bundles USING gin (search);
CREATE INDEX bundles_name_trgm ON bundles USING gin (name gin_trgm_ops);
CREATE INDEX bundles_creator_idx ON bundles (creator_id);
