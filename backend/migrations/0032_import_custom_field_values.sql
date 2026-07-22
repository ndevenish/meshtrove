-- Custom field values on an *import*.
--
-- The import page types the drop's metadata before the model or bundle it
-- describes exists, so a scalar field can wait: it rides along in the commit
-- body and is written once there is an owner. A file-kind field can't — its
-- payload is bytes, and bytes need somewhere to live the moment they are
-- dropped. So the import becomes a third owner of a value, exactly as it is a
-- third owner of a file: the value is staged on the import and copied onto
-- whatever the commit carves out of it.
--
-- Any kind may be stored here, not just `file` — a split has to carry the whole
-- of what was typed into the import it lifts out, and nothing is gained by
-- teaching it that some kinds don't travel.

ALTER TABLE custom_field_values ADD COLUMN import_id uuid
    REFERENCES imports (id) ON DELETE CASCADE;
ALTER TABLE custom_field_values DROP CONSTRAINT cfv_one_owner;
ALTER TABLE custom_field_values ADD CONSTRAINT cfv_one_owner
    CHECK (num_nonnulls(model_id, bundle_id, import_id) = 1);

CREATE UNIQUE INDEX cfv_import_uq ON custom_field_values (field_id, import_id)
    WHERE import_id IS NOT NULL;
CREATE INDEX cfv_by_import ON custom_field_values (import_id) WHERE import_id IS NOT NULL;
