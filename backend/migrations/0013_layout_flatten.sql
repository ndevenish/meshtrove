-- "Discard folders" becomes part of a saved layout, not a per-import toggle:
-- a layout that knows how to carve a publisher's tree also knows whether the
-- folders are worth keeping once carved. Loot's scale/support folders are spent
-- the moment the carve reads them, so its layout flattens by default.
ALTER TABLE import_layouts
    ADD COLUMN flatten boolean NOT NULL DEFAULT false;

UPDATE import_layouts SET flatten = true WHERE name = 'Loot Studios collection';

-- Humanising a captured value splits its camelCase before it is matched, so the
-- old no-separator "nosupports" key can no longer match the "NoSupports" folder.
-- Re-key it the way the folder spells it (any separator style now folds alike).
UPDATE import_layouts
SET value_map = (value_map - 'nosupports') || '{"no supports": ["unsupported"]}'::jsonb
WHERE name = 'Loot Studios collection'
  AND value_map ? 'nosupports';
