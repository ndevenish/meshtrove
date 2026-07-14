-- ---------------------------------------------------------------------------
-- What picture to show for a model, and for a bundle.
--
-- An image is attached where it was *rendered*: a preview of an STL that a carve
-- put on a variant is an image of that variant (files/renderer.rs). But the cards
-- asked for a model-level image — `images WHERE model_id = m.id` — and a carved
-- model owns no files directly, so it owns no images either. Result: 43 render
-- jobs succeed, 42 images land on variants, and every card in the library is
-- blank. The images were never missing; nothing was looking where they are.
--
-- A container therefore falls back to what is inside it: a model to its variants,
-- a bundle to its member models. Its own primary image still wins when it has
-- one — an uploaded box-art shot beats a render of the first knight.
--
-- Two functions rather than five copies of the same subquery: browse, search,
-- the bundle list and the bundle detail all asked this question separately, and
-- that is exactly how they drift.
-- ---------------------------------------------------------------------------

CREATE FUNCTION model_preview_image(model uuid) RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT id FROM (
        -- The model's own primary image, if it has one.
        SELECT i.id, 0 AS tier, i.sort_order, i.created_at
        FROM images i
        WHERE i.model_id = model AND i.is_primary
        UNION ALL
        -- Otherwise the primary image of one of its variants — where a carved
        -- model's renders actually live.
        SELECT i.id, 1 AS tier, i.sort_order, i.created_at
        FROM images i
        JOIN model_variants v ON v.id = i.variant_id
        WHERE v.model_id = model AND i.is_primary
    ) x
    ORDER BY tier, sort_order, created_at
    LIMIT 1
$$;

CREATE FUNCTION bundle_preview_image(bundle uuid) RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT id FROM (
        SELECT i.id, 0 AS tier, i.sort_order, i.created_at
        FROM images i
        WHERE i.bundle_id = bundle AND i.is_primary
        UNION ALL
        -- Otherwise borrow from a member: whatever the first model in the box
        -- would show, the box shows.
        SELECT model_preview_image(bm.model_id), 1 AS tier, 0, m.created_at
        FROM bundle_models bm
        JOIN models m ON m.id = bm.model_id
        WHERE bm.bundle_id = bundle AND model_preview_image(bm.model_id) IS NOT NULL
    ) x
    ORDER BY tier, sort_order, created_at
    LIMIT 1
$$;
