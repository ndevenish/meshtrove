-- ---------------------------------------------------------------------------
-- Widen the seeded "Loot Studios collection" layout pattern.
--
-- A new migration, not an edit to 0005: that one is applied and checksummed
-- everywhere, and changing it in place makes `sqlx migrate` refuse to run. So the
-- seed is updated where it can be — here.
--
-- The old pattern assumed every model folder was `Name_32mm_Support/…stl`. Real
-- archives are looser, so the pattern now also handles:
--   * a version suffix on the name  — `WarriorMummy_V2_75mm_…`  → name keeps the _V2
--   * no scale at all               — `Obelisk_Supported/…`     → scale group empty
--   * any file extension            — `.lys`, `.chitubox`, …    → not just `.stl`
-- Roles and value map are unchanged: group 1 model tag, 2 model name, 3 + 4
-- variant tags. An empty scale group simply contributes no tag (layout.rs skips
-- an unmatched optional group).
-- ---------------------------------------------------------------------------

UPDATE import_layouts
SET pattern = '(?i)^(?:[^/]+/)*?\d+ - ([^/]+)/[^/]+/([^/]+?(?:_V\d)?)_(\d+mm)?_?([^/]+)/[^/]+\..+$'
WHERE name = 'Loot Studios collection';
