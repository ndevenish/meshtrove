-- Carry over the "Supported Holllow" rule (three L's) from the tuned Loot
-- template: some archives really do spell the hollow folder that way, and it
-- should tag the same as "supported hollow". Added, not folded into 0018, which
-- is already applied and checksummed.
UPDATE import_layouts
SET value_map = value_map || '{"supported holllow": ["supported", "hollow"]}'::jsonb
WHERE name = 'Loot Studios collection';
