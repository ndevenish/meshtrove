-- ---------------------------------------------------------------------------
-- Let the seeded "Loot Studios collection" layout reach into `autosave` folders.
--
-- A new migration, not an edit to 0011: applied migrations are checksummed, so
-- the seed is updated where it can be — here (same reasoning as 0011).
--
-- Some Loot archives drop the sliceable files a level deeper, inside an
-- `autosave` subfolder of the variant folder:
--   .../Gold_32mm_Supported/autosave/Gold.chitubox
-- The old pattern's final segment (`[^/]+\..+$`) stopped at the variant folder
-- and never saw them. Allow an optional `autosave` folder between the variant
-- folder and the file. Roles, value map, and the four capture groups are all
-- unchanged — the new group is non-capturing.
-- ---------------------------------------------------------------------------

UPDATE import_layouts
SET pattern = '(?i)^(?:[^/]+/)*?\d+ - ([^/]+)/[^/]+/([^/]+?(?:_V\d)?)_(\d+mm)?_?([^/]+)/(?:[^/]*autosave\/)?[^/]+\..+$'
WHERE name = 'Loot Studios collection';
