-- Windows folder cruft (Thumbs.db thumbnail caches, desktop.ini folder settings)
-- rode in on archives before `is_os_junk` learned to skip it. Remove what is
-- already stored, matching the filter: by filename at any depth, ignoring case.
--
-- Files attached to a custom field are left alone: an attachment is something a
-- person picked deliberately, not something unpacked from an archive.
--
-- Only the `files` rows go. The blobs stay in the store until the orphan GC
-- sweeps them, and `images.source_file_id` is `ON DELETE SET NULL`.

-- `owner_model` is the model a file belongs to, directly or through its variant,
-- captured now because that variant may be gone by the time we report.
CREATE TEMP TABLE junk_files ON COMMIT DROP AS
SELECT f.id, f.variant_id, f.bundle_id, f.import_id,
       coalesce(f.model_id, v.model_id) AS owner_model
FROM files f
LEFT JOIN model_variants v ON v.id = f.variant_id
WHERE lower(f.filename) IN ('thumbs.db', 'desktop.ini')
  AND f.custom_field_value_id IS NULL;

DELETE FROM files f USING junk_files j WHERE f.id = j.id;

-- A variant that held nothing but junk is now empty. Carve's rule applies here
-- too (routes/carve.rs): it goes only if nothing else is lost with it, meaning
-- no name, no print notes and no pictures. A variant someone labelled or
-- illustrated stays, even with no files left. Only variants this purge emptied
-- are considered; variants that were already empty are none of its business.
--
-- Models, bundles and imports are never deleted: they carry names, tags,
-- descriptions and provenance. Those left with no files are reported so they
-- can be reviewed by hand.
DO $$
DECLARE
    n_files    bigint;
    n_variants bigint;
    r          record;
BEGIN
    SELECT count(*) INTO n_files FROM junk_files;

    DELETE FROM model_variants v
    WHERE v.id IN (SELECT variant_id FROM junk_files WHERE variant_id IS NOT NULL)
      AND v.name IS NULL AND v.print_notes IS NULL
      AND NOT EXISTS (SELECT 1 FROM files f WHERE f.variant_id = v.id)
      AND NOT EXISTS (SELECT 1 FROM images i WHERE i.variant_id = v.id);
    GET DIAGNOSTICS n_variants = ROW_COUNT;

    RAISE NOTICE 'purged % Windows junk file(s); removed % emptied variant(s)',
        n_files, n_variants;

    FOR r IN
        SELECT DISTINCT m.id, m.name
        FROM junk_files j JOIN models m ON m.id = j.owner_model
        WHERE NOT EXISTS (SELECT 1 FROM files f WHERE f.model_id = m.id)
          AND NOT EXISTS (
              SELECT 1 FROM files f JOIN model_variants mv ON mv.id = f.variant_id
              WHERE mv.model_id = m.id)
    LOOP
        RAISE WARNING 'model % (%) has no files left after the junk purge', r.id, r.name;
    END LOOP;

    FOR r IN
        SELECT DISTINCT b.id, b.name
        FROM junk_files j JOIN bundles b ON b.id = j.bundle_id
        WHERE NOT EXISTS (SELECT 1 FROM files f WHERE f.bundle_id = b.id)
    LOOP
        RAISE WARNING 'bundle % (%) has no files left after the junk purge', r.id, r.name;
    END LOOP;

    FOR r IN
        SELECT DISTINCT i.id
        FROM junk_files j JOIN imports i ON i.id = j.import_id
        WHERE NOT EXISTS (SELECT 1 FROM files f WHERE f.import_id = i.id)
    LOOP
        RAISE WARNING 'import % has no files left after the junk purge', r.id;
    END LOOP;
END $$;
