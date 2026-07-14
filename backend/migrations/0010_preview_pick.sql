-- ---------------------------------------------------------------------------
-- Which variant speaks for the model.
--
-- Every variant renders its own thumbnail now, so a model has several to choose
-- from and the choice is no longer "whichever came first". Prefer the *plainest*
-- variant: fewest tags is the one carrying the fewest qualifiers — the bare
-- 32mm, not the 32mm-supported-lychee-presupported edition — and it is the one
-- that looks like the model rather than like a printing option. Ties break on
-- the shortest source filename, on the same instinct: `knight.stl` is the
-- knight, `knight_base_v2_hollow.stl` is a detail of it.
--
-- The model's own primary image still wins outright when it has one — that is
-- what "promote to model" (a favourited variant shot) sets.
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION model_preview_image(model uuid) RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT id FROM (
        -- The model's own primary image: an uploaded shot, or one promoted from
        -- a variant. Beats every render.
        SELECT i.id,
               0 AS tier,
               0 AS tags,
               0 AS name_len,
               i.created_at
        FROM images i
        WHERE i.model_id = model AND i.is_primary
        UNION ALL
        -- Otherwise the plainest variant's picture.
        SELECT i.id,
               1 AS tier,
               (SELECT count(*) FROM variant_tag_assignments a WHERE a.variant_id = v.id) AS tags,
               coalesce(length(f.filename), 0) AS name_len,
               i.created_at
        FROM images i
        JOIN model_variants v ON v.id = i.variant_id
        LEFT JOIN files f ON f.id = i.source_file_id
        WHERE v.model_id = model AND i.is_primary
    ) x
    ORDER BY tier, tags, name_len, created_at
    LIMIT 1
$$;
