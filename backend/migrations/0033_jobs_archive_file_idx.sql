-- The import file listing (`list_import_files`) resolves each staged file's
-- unpack state through a LATERAL over `jobs`, matched on
-- `payload->>'archive_file_id'`. That expression had no index, so every row of
-- the listing drove its own sequential scan of `jobs` — fine at a hundred files,
-- quadratic at forty thousand. On a large dropbox pickup the query reached ~20s
-- and the import page polls it, so the server spent whole cores re-answering it
-- back to back while the import it was reporting on starved.
--
-- IF NOT EXISTS because the index was first applied by hand on a live server to
-- stop the bleeding; this migration has to reconcile with that rather than fail
-- against it.
--
-- Not CONCURRENTLY: sqlx runs each migration inside a transaction, which forbids
-- it. Building it takes a lock on `jobs`, but only for as long as the table
-- takes to scan at startup, before any worker is claiming from it.
CREATE INDEX IF NOT EXISTS jobs_archive_file
    ON jobs ((payload->>'archive_file_id'))
    WHERE kind = 'import_archive';
