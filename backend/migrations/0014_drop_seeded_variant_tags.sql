-- Start with no known variant tags.
--
-- The initial migration seeded a starter scale/support vocabulary (32mm, 75mm,
-- supported, unsupported, supported_hollow, lychee_project, merged), carried
-- into variant_tags by 0004. But the vocabulary is meant to grow from what you
-- actually import — a fresh archive asserting "32mm" and "supported" as things
-- that exist, before a single file has claimed them, is a guess the schema has
-- no business making. Drop the seed.
--
-- Only where it is still untouched: on an established database some of these
-- names have long since been claimed by real variants (and share their identity
-- via tag_key), so deleting them would cascade their assignments away. Those we
-- leave — they are no longer seed data, they are the vocabulary in use. On a
-- fresh database none are claimed, so all seven go and the vocabulary is empty.
DELETE FROM variant_tags vt
WHERE vt.name IN (
        '32mm', '75mm',
        'unsupported', 'supported', 'supported_hollow', 'lychee_project', 'merged'
      )
  AND NOT EXISTS (
        SELECT 1 FROM variant_tag_assignments a WHERE a.tag_id = vt.id
      );
