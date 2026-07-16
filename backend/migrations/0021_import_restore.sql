-- A dropped archive that turns out to be a MeshTrove export (it carries a
-- manifest.json) is not carved like an unknown folder — it is *restored*,
-- recreating the models and bundles it holds with their identity intact. The
-- upload detects this at drop time (a cheap read of the zip's central directory)
-- and sets this flag instead of queueing the usual unpack job; the Import page
-- then offers "restore" rather than the carve/layout UI.
ALTER TABLE imports
    ADD COLUMN is_export boolean NOT NULL DEFAULT false;
