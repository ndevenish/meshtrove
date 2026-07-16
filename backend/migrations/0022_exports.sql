-- Exports: a per-user, asynchronously-built archive.
--
-- A bundle can be gigabytes, so building its zip is a background job (like the
-- import unpack), not a request that blocks until the bytes are ready. An export
-- row records what to build (`spec`) and tracks the job's progress; once the job
-- finishes it holds the size of the finished file, which lives outside the
-- content-addressed blob store (it is a one-off artifact, not a shared blob) at
-- <store>/exports/<id>.zip. The user downloads it from the Exports page and
-- deletes it when done.
CREATE TABLE exports (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Display label and the basis of the download filename.
    name       text NOT NULL,
    created_by uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- building | ready | failed. Driven by the export_archive job.
    status     text NOT NULL DEFAULT 'building',
    -- What to gather: {bundle_id?, model_ids[], variant_include[], variant_exclude[]}.
    spec       jsonb NOT NULL,
    -- Set when status='ready'.
    size       bigint,
    filename   text,
    -- Set when status='failed'.
    error      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX exports_by_creator ON exports (created_by, created_at DESC);
