-- Alternate names a model is known by. Scraped-metadata imports carry the
-- source's spellings for each model, and renaming a model through a merge should
-- not lose the name it had — both land here so a later import still resolves the
-- model by any name it has ever gone by.
--
-- citext + composite primary key does the deduplication the importer needs for
-- free: "Gold", "gold" and "GOLD" collapse to one row per model, and a plain
-- INSERT ... ON CONFLICT DO NOTHING skips a repeat.
CREATE TABLE model_aliases (
    model_id   uuid   NOT NULL REFERENCES models (id) ON DELETE CASCADE,
    alias      citext NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (model_id, alias)
);

-- Lookups go alias -> model (does any model already answer to this name?).
CREATE INDEX model_aliases_alias_idx ON model_aliases (alias);
