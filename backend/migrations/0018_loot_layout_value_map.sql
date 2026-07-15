-- Bring the seeded "Loot Studios collection" value map in line with the tuned
-- mapping worked out against real archives (the user's "Loot New" template):
--   * NoSupports maps to no tag at all (unsupported is the default, unstated)
--   * the lychee edition is just "supported" — lychee_project is dropped
--   * hollow prints are tagged "hollow", not "supported_hollow"
-- Keys are canonical (humanised) so a carve and a UI edit key them alike.
UPDATE import_layouts
SET value_map = '{
        "32mm": ["32mm"],
        "75mm": ["75mm"],
        "supported": ["supported"],
        "no supports": [],
        "unsupported": ["unsupported"],
        "supported solid": ["supported"],
        "supported hollow": ["supported", "hollow"],
        "supported lychee": ["supported"]
    }'::jsonb
WHERE name = 'Loot Studios collection';
