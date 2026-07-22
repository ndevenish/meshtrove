-- How far along a long job is, for the jobs that care to say.
--
-- An unpack or a dropbox pickup spends its time in a loop over files: hash into
-- the store, insert a row, repeat, for anything up to tens of thousands of
-- entries. Until now the only thing an admin could watch was the staged file
-- count climbing with no idea of what it was climbing towards, which on a
-- multi-hour pickup reads as a hang.
--
-- Both nullable, and independently so:
--   * both null — the job reports nothing (a render), or hasn't started saying.
--   * done set, total null — counting up with no known end. That is honest for
--     the phase before an archive has been opened: only zip carries an index
--     cheap enough to read ahead of the extraction.
--   * both set — a denominator worth drawing a bar with.
--
-- Deliberately not a percentage: the caller knows files, and a percentage
-- computed at the source can't be re-rendered as "412 of 1,203".
ALTER TABLE jobs
    ADD COLUMN progress_done  int,
    ADD COLUMN progress_total int;

-- What is still being done to one import, in one pass over `jobs`: whether
-- anything is running on it at all, how many of its archives are still to be
-- opened, and how far the running jobs have got through the files they are
-- staging.
--
-- A function because both the Importing list and the single-import fetch need
-- exactly this, and it is the query that has already been a performance
-- problem twice — it is worth there being one copy of it to fix.
--
-- Driven from jobs, not from files. Only a handful of jobs are ever queued or
-- running, while an import can hold tens of thousands of files; joined the
-- other way round the two have no predicate relating them but the payload
-- match, so the pair is a cartesian product and concluding "nothing is
-- unpacking" means walking the whole of it. That is what put the Importing
-- list — polled every 3s — into seconds on a large import. The payload is
-- serialised from a Uuid (see importer.rs), so the cast is total, and it puts
-- the lookup on the files primary key.
--
-- STABLE and a plain SQL body, so the planner inlines it into the caller's
-- LATERAL rather than treating it as a black box called per row.
CREATE FUNCTION import_work(import uuid)
RETURNS TABLE (
    jobs_left     bigint,
    archives_left bigint,
    staging_done  bigint,
    staging_total bigint
)
LANGUAGE sql STABLE AS $$
    SELECT count(*),
           count(*) FILTER (WHERE j.kind = 'import_archive'),
           coalesce(sum(j.progress_done), 0),
           coalesce(sum(j.progress_total), 0)
    FROM jobs j
    LEFT JOIN files f ON f.id = (j.payload->>'archive_file_id')::uuid
    WHERE j.status IN ('queued', 'running')
      AND ((j.kind = 'import_archive' AND f.import_id = import)
        OR (j.kind = 'dropbox_import' AND j.payload->>'import_id' = import::text));
$$;
