-- ---------------------------------------------------------------------------
-- Which *bytes* a preview image is currently made of.
--
-- A card works from a summary, which carries only the preview image's id — and
-- an id is no longer a promise about content: re-rendering rewrites the row in
-- place (same id, new blob) precisely so the page can keep holding it. So a
-- card's `/api/images/{id}/square` URL is stable across a change that was made
-- to correct the picture, and a browser that cached it keeps showing the old
-- render. The detail pages don't have this problem: they read the full image
-- record and append `?v=<blob_sha256>`, so their URL moves with the bytes.
--
-- This is the missing half of that trick for the summaries: compose it with
-- either preview picker — `image_version(model_preview_image(m.id))` — to get
-- the version string beside the id, so cards can name their bytes too.
-- ---------------------------------------------------------------------------

CREATE FUNCTION image_version(image uuid) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT blob_sha256 FROM images WHERE id = image
$$;
