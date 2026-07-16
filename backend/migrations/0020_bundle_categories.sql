-- A bundle's "core categories": the import's section folders (Heroes, Enemies,
-- NPC, …) kept as an explicit, ordered list rather than re-guessed from the
-- title-cased member tags on every render. A category IS a model tag; a member
-- belongs to it by carrying that tag. `position` is the tab order.
CREATE TABLE bundle_categories (
    bundle_id uuid NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    tag_id    uuid NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position  int  NOT NULL,
    PRIMARY KEY (bundle_id, tag_id)
);
CREATE INDEX bundle_categories_by_bundle ON bundle_categories (bundle_id, position);

-- Backfill existing bundles from the heuristic the UI used until now: the
-- title-cased member model tags (Heroes, Enemies — as opposed to lowercase
-- descriptive tags like undead, medium), ordered by how many members carry
-- each. The import's folder order isn't recoverable for already-imported
-- bundles, so size order is the best seed; it's editable from the bundle page.
INSERT INTO bundle_categories (bundle_id, tag_id, position)
SELECT bundle_id, tag_id,
       (row_number() OVER (PARTITION BY bundle_id ORDER BY cnt DESC, name))::int - 1
FROM (
    SELECT bm.bundle_id, mt.tag_id, t.name::text AS name, count(*) AS cnt
    FROM bundle_models bm
    JOIN model_tags mt ON mt.model_id = bm.model_id
    JOIN tags t ON t.id = mt.tag_id
    WHERE left(t.name::text, 1) <> lower(left(t.name::text, 1))
    GROUP BY bm.bundle_id, mt.tag_id, t.name
) x;
