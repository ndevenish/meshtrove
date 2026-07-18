-- A layout becomes a *list* of regex rules rather than one regex.
--
-- One pattern that has to capture the model name, the category, the scale and
-- the support state all at once is fragile to write and impossible to reuse. A
-- layout is now several small patterns, each searched across the path, whose
-- captured model tags and variant tags are merged (services/layout.rs). Each
-- rule carries its own roles + value_map (a group number only ever means
-- something within its own pattern) plus a label and an on/off switch.
--
-- The old single-pattern columns fold into a one-element `rules` array, which is
-- exactly equivalent: the seeded patterns are self-anchored (^…$), so searching
-- instead of anchoring finds the same single match.

ALTER TABLE import_layouts ADD COLUMN rules jsonb NOT NULL DEFAULT '[]'::jsonb;

UPDATE import_layouts
   SET rules = jsonb_build_array(jsonb_build_object(
           'name', '',
           'pattern', pattern,
           'roles', roles,
           'value_map', value_map,
           'enabled', true));

ALTER TABLE import_layouts
    DROP COLUMN pattern,
    DROP COLUMN roles,
    DROP COLUMN value_map;
