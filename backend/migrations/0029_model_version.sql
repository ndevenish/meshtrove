-- The creator's version for a model — e.g. "v2", "2024 rework", the revision a
-- publisher re-cuts a sculpt under. Free text: versions are whatever the
-- creator calls them, not an ordering we can impose. Editable on the model
-- page, and fillable on import: a carve layout can assign a capture group the
-- `version` role to pull it out of the path.
-- Named `model_version`, not `version`: a bare `version` column reads as row
-- versioning / optimistic locking, which this is not.
ALTER TABLE models ADD COLUMN model_version text;
