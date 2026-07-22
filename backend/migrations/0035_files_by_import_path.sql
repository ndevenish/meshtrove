-- The import page now opens one folder at a time (`GET /api/imports/{id}/files?path=`)
-- rather than pulling every staged file, because a dropbox pickup of a
-- creator's back catalogue runs to tens of thousands of them.
--
-- `files_by_import` alone would answer that by walking every row of the import
-- and discarding all but one folder's worth — on a 42k-file import, 42k index
-- entries read to return twenty. Carrying `path` in the index makes the folder
-- the thing looked up.
--
-- It replaces `files_by_import` rather than joining it: the leading column is
-- the same, so the composite serves the plain `import_id` lookups too.
CREATE INDEX files_by_import_path ON files (import_id, path) WHERE import_id IS NOT NULL;
DROP INDEX files_by_import;
