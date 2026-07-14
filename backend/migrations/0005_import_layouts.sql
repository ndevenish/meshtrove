-- Import layout templates: a saved regex whose capture groups carve a staged
-- import into models / variants at commit time (see docs/plan.md, "Import
-- layout templates — regex-driven carve").
--
-- `pattern` is a fancy-regex matched against each staged file's full logical
-- path (path/filename, implicitly anchored). `roles` assigns each capture
-- group a meaning; `value_map` normalises raw captures into variant-tag names
-- ("Supported_LYCHEE" -> supported + lychee_project). Both are data — the
-- backend engine is the single interpreter, the frontend never runs the regex.

CREATE TABLE import_layouts (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name       citext NOT NULL UNIQUE,
    pattern    text NOT NULL,
    -- {"<group number>": "model_name" | "model_tag" | "variant_tag" | "ignore"}
    roles      jsonb NOT NULL DEFAULT '{}'::jsonb,
    -- {"<lowercased raw capture>": ["variant tag name", ...]} — an empty list
    -- means "recognised, maps to no tags".
    value_map  jsonb NOT NULL DEFAULT '{}'::jsonb,
    -- The publisher whose archives this layout fits, for auto-suggestion.
    creator_id uuid REFERENCES creators (id) ON DELETE SET NULL,
    -- NULL = shipped preset (seeded below, before any user exists).
    created_by uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- The Loot Studios collection layout (docs/import-layouts.md, layout B):
--   DownloadAll_32mm/1 - Heroes/BuriedTombHeroes_32mm_Supported_LYCHEE/
--     Gold_32mm_Supported_Lychee/*.stl
-- category -> model tag, mini -> model name, scale + support -> variant tags.
INSERT INTO import_layouts (name, pattern, roles, value_map) VALUES (
    'Loot Studios collection',
    '(?i)^(?:[^/]+/)*?\d+ - ([^/]+)/[^/]+/([^/]+?)_(\d+mm)_([^/]+)/[^/]+\.stl$',
    '{"1": "model_tag", "2": "model_name", "3": "variant_tag", "4": "variant_tag"}',
    '{"32mm": ["32mm"], "75mm": ["75mm"],
      "nosupports": ["unsupported"], "unsupported": ["unsupported"],
      "supported": ["supported"],
      "supported_lychee": ["supported", "lychee_project"],
      "supported_solid": ["supported"],
      "supported_hollow": ["supported", "supported_hollow"]}'
);
