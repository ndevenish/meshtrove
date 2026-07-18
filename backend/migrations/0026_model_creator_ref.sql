-- The creator's own id/SKU for a model — e.g. the product code a publisher
-- ships it under. Free text, distinct from `creator_id` (the FK to the creators
-- table). Editable on the model page, and fillable on import: a carve layout can
-- assign a capture group the `creator_ref` role to pull it out of the path.
ALTER TABLE models ADD COLUMN creator_ref text;
