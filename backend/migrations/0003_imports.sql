-- Imports: a staging area for a dropped archive.
--
-- A dropped file lands in an `import` — a holding object that is deliberately
-- NOT a model and NOT a bundle. It never appears in browse/search; it is only
-- listed on the "Importing" page. Once its archive has unpacked, the user
-- commits it to exactly one destination (new model / new bundle / existing
-- bundle), which moves its files onto that owner and drops the import row.
--
-- This is what removes the guess at drop time: nothing has to decide "is this
-- one model or a collection?" while bytes are still uploading, and there is no
-- model<->bundle conversion afterwards.

CREATE TABLE imports (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Seeded from the archive filename; editable before commit and used as the
    -- default name for the model/bundle it becomes.
    name       text NOT NULL,
    created_by uuid NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX imports_by_creator ON imports (created_by);

-- A file may now also be owned by an import (the staging bucket), still exactly
-- one owner — so every existing query that filters on model/variant/bundle keeps
-- working unchanged, and staged files cannot leak into the library.
ALTER TABLE files ADD COLUMN import_id uuid REFERENCES imports (id) ON DELETE CASCADE;
ALTER TABLE files DROP CONSTRAINT files_check;
ALTER TABLE files ADD CONSTRAINT files_one_owner
    CHECK (num_nonnulls(model_id, variant_id, bundle_id, import_id) = 1);
CREATE INDEX files_by_import ON files (import_id) WHERE import_id IS NOT NULL;
