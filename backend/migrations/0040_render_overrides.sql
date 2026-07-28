-- ---------------------------------------------------------------------------
-- Per-file render overrides: which way is up, and which way it faces.
--
-- The renderer is configured globally (settings.renderer), but orientation is
-- not a global fact — it is a property of the *mesh*. f3d assumes +Y up unless
-- told otherwise, and print files are authored by whoever made them: a Z-up STL
-- rendered with a Y-up camera comes out lying on its side. No single global
-- value fixes a library where the files disagree with each other.
--
-- So the override lives on the file, not on the picture. Re-render the image
-- and the fix comes with it; run the admin's bulk re-render and every fix
-- survives, because the job reads the file every time rather than replaying
-- whatever the global config happened to be.
--
-- Shape: {"up": "+Z", "turntable": 135}. `up` is an f3d axis (+X/-X/+Y/…);
-- `turntable` is a camera azimuth in degrees about that up axis. Absent keys
-- mean "whatever the global config says", which is why this is a nullable jsonb
-- rather than a pair of columns with defaults.
-- ---------------------------------------------------------------------------

ALTER TABLE files ADD COLUMN render_overrides jsonb;

-- What actually produced this picture. `renderer_config` records the global
-- config; this records the per-file override layered on top, kept apart so the
-- "is it stale?" comparison against the current global config still means what
-- it says (an overridden image is not stale merely for being overridden).
ALTER TABLE images ADD COLUMN render_overrides jsonb;
