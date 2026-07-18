-- Ship the "Loot Nautilus" carve as a default layout preset, alongside "Loot
-- Studios collection" (0005). Nautilus box sets fold scale, support state and
-- the FDM edition into the filename rather than into folders, so one all-in-one
-- pattern won't carve them; this layout is a list of small rules (0023), each
-- finding one fragment of the path/filename and merging its captures:
--   Category  — /All_<Category>            -> model tag
--   Size      — _<NN>mm_                    -> variant tag
--   Support   — (Un)Supported(_Hollow)      -> variant tag(s)
--   Name      — /<Name>_…                   -> model name
--   FDM       — FDM                          -> variant tag
--
-- Built with jsonb_build_* so the regex backslashes need no JSON escaping. Seeds
-- with created_by/creator_id NULL, marking it a shipped preset. ON CONFLICT does
-- nothing so an install where someone already saved a layout by this name keeps
-- theirs untouched.
INSERT INTO import_layouts (name, rules, flatten)
VALUES (
    'Loot Nautilus',
    jsonb_build_array(
        jsonb_build_object(
            'name', 'Category',
            'pattern', '/All_([^_]+)',
            'roles', '{"1": "model_tag", "2": "model_tag"}'::jsonb,
            'value_map', '{}'::jsonb,
            'enabled', true
        ),
        jsonb_build_object(
            'name', 'Size',
            'pattern', '_(\d\dmm)_',
            'roles', '{"1": "variant_tag"}'::jsonb,
            'value_map', '{}'::jsonb,
            'enabled', true
        ),
        jsonb_build_object(
            'name', 'Support Variants',
            'pattern', '(?i)((?:Un)?Supported(?:_Hollow)?)',
            'roles', '{"1": "variant_tag"}'::jsonb,
            'value_map', jsonb_build_object(
                'supported hollow', jsonb_build_array('supported', 'hollow'),
                'un supported', jsonb_build_array()
            ),
            'enabled', true
        ),
        jsonb_build_object(
            'name', 'Name',
            'pattern', '/([^_]+)_[^/]+$',
            'roles', '{"1": "model_name"}'::jsonb,
            'value_map', '{}'::jsonb,
            'enabled', true
        ),
        jsonb_build_object(
            'name', '',
            'pattern', '(FDM)',
            'roles', '{"1": "variant_tag"}'::jsonb,
            'value_map', '{}'::jsonb,
            'enabled', true
        )
    ),
    true
)
ON CONFLICT (name) DO NOTHING;
