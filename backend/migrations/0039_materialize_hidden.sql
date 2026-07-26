-- Materialize "is this item hidden" onto models and bundles.
--
-- An item is hidden iff it carries at least one tag flagged `hidden` (see
-- 0038_hidden_tags). That predicate used to be recomputed inline as a
-- `NOT EXISTS (… JOIN tags … WHERE hidden)` subquery in half a dozen places —
-- browse, search, the tag cloud, bundle member lists — and, crucially, it was
-- *missing* from the derived surfaces: `bundle_preview_image` happily borrowed a
-- hidden member's artwork for a bundle thumbnail, and a bundle's model_count
-- counted members a visitor could never see.
--
-- Reads (browse) are frequent; the writes that change hidden-ness (an admin
-- flipping a tag's `hidden`, or a model being (un)tagged) are rare. So we pay at
-- write time: a boolean column kept current by triggers, read as a plain `NOT
-- m.hidden` everywhere. One source of truth for the whole app.

ALTER TABLE models  ADD COLUMN hidden boolean NOT NULL DEFAULT false;
ALTER TABLE bundles ADD COLUMN hidden boolean NOT NULL DEFAULT false;

-- Partial indexes: the common query wants the *visible* rows, and hidden ones
-- are the rare minority, so index the exception.
CREATE INDEX models_hidden_idx  ON models  (id) WHERE hidden;
CREATE INDEX bundles_hidden_idx ON bundles (id) WHERE hidden;

-- Backfill from the current tag associations.
UPDATE models m SET hidden = EXISTS (
    SELECT 1 FROM model_tags mt JOIN tags t ON t.id = mt.tag_id
    WHERE mt.model_id = m.id AND t.hidden
);
UPDATE bundles b SET hidden = EXISTS (
    SELECT 1 FROM bundle_tags bt JOIN tags t ON t.id = bt.tag_id
    WHERE bt.bundle_id = b.id AND t.hidden
);

-- --------------------------------------------------------------------------
-- Recompute triggers. Hidden-ness moves when either side of the model↔tag (or
-- bundle↔tag) relationship changes:
--   * a tag's `hidden` flag flips        -> every item carrying that tag
--   * a model/bundle is (un)tagged       -> that one item
-- Both are handled; missing the second leaves stale flags on the *more common*
-- operation (tagging), which is exactly the bug this column exists to prevent.
-- --------------------------------------------------------------------------

-- One membership row changed: recompute just the affected item. On a CASCADE
-- delete of the item itself the row is already gone, so the UPDATE is a no-op.
CREATE FUNCTION model_tags_sync_hidden() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE models m SET hidden = EXISTS (
        SELECT 1 FROM model_tags mt JOIN tags t ON t.id = mt.tag_id
        WHERE mt.model_id = m.id AND t.hidden
    ) WHERE m.id = COALESCE(NEW.model_id, OLD.model_id);
    RETURN NULL;
END $$;
CREATE TRIGGER model_tags_sync_hidden
    AFTER INSERT OR DELETE ON model_tags
    FOR EACH ROW EXECUTE FUNCTION model_tags_sync_hidden();

CREATE FUNCTION bundle_tags_sync_hidden() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE bundles b SET hidden = EXISTS (
        SELECT 1 FROM bundle_tags bt JOIN tags t ON t.id = bt.tag_id
        WHERE bt.bundle_id = b.id AND t.hidden
    ) WHERE b.id = COALESCE(NEW.bundle_id, OLD.bundle_id);
    RETURN NULL;
END $$;
CREATE TRIGGER bundle_tags_sync_hidden
    AFTER INSERT OR DELETE ON bundle_tags
    FOR EACH ROW EXECUTE FUNCTION bundle_tags_sync_hidden();

-- A tag's visibility flipped: recompute every model and bundle carrying it.
-- WHEN () keeps this off the hot path — retagging etc. never touches `hidden`.
CREATE FUNCTION tag_sync_hidden() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE models m SET hidden = EXISTS (
        SELECT 1 FROM model_tags mt JOIN tags t ON t.id = mt.tag_id
        WHERE mt.model_id = m.id AND t.hidden
    ) WHERE m.id IN (SELECT model_id FROM model_tags WHERE tag_id = NEW.id);

    UPDATE bundles b SET hidden = EXISTS (
        SELECT 1 FROM bundle_tags bt JOIN tags t ON t.id = bt.tag_id
        WHERE bt.bundle_id = b.id AND t.hidden
    ) WHERE b.id IN (SELECT bundle_id FROM bundle_tags WHERE tag_id = NEW.id);
    RETURN NULL;
END $$;
CREATE TRIGGER tag_sync_hidden
    AFTER UPDATE OF hidden ON tags
    FOR EACH ROW WHEN (OLD.hidden IS DISTINCT FROM NEW.hidden)
    EXECUTE FUNCTION tag_sync_hidden();

-- --------------------------------------------------------------------------
-- Teach the bundle preview to respect member visibility. The borrow tier now
-- skips hidden members unless the caller asks to include them (an admin, who
-- sees hidden members on the bundle page anyway). Signature changes, so drop
-- and recreate rather than CREATE OR REPLACE.
-- --------------------------------------------------------------------------
DROP FUNCTION bundle_preview_image(uuid);
CREATE FUNCTION bundle_preview_image(bundle uuid, include_hidden boolean DEFAULT false)
RETURNS uuid LANGUAGE sql STABLE AS $$
    SELECT id FROM (
        SELECT i.id, 0 AS tier, i.sort_order, i.created_at
        FROM images i
        WHERE i.bundle_id = bundle AND i.is_primary
        UNION ALL
        -- Otherwise borrow from a *visible* member: whatever the first model a
        -- viewer can actually see in the box would show, the box shows.
        SELECT model_preview_image(bm.model_id), 1 AS tier, 0, m.created_at
        FROM bundle_models bm
        JOIN models m ON m.id = bm.model_id
        WHERE bm.bundle_id = bundle
          AND (include_hidden OR NOT m.hidden)
          AND model_preview_image(bm.model_id) IS NOT NULL
    ) x
    ORDER BY tier, sort_order, created_at
    LIMIT 1
$$;
