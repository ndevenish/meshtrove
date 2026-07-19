-- The tag/creator rename-refresh triggers re-vectorize every model and bundle
-- carrying the renamed row. But `UPDATE OF name` fires on the column being
-- *assigned*, not on it changing — and upsert_tag's race-safe
-- `ON CONFLICT (name) DO UPDATE SET name = tags.name` assigns it on every
-- existing tag. A metadata import applying common tags to many models thus
-- rebuilt the search vector of every model in the library per tag applied,
-- turning a bulk apply into minutes of trigger work. Gate all four rename
-- triggers on the name actually changing (::text so a case-only rename of the
-- citext still counts as a change, harmlessly).

CREATE OR REPLACE TRIGGER tags_rename_refresh
    AFTER UPDATE OF name ON tags
    FOR EACH ROW
    WHEN (OLD.name::text IS DISTINCT FROM NEW.name::text)
    EXECUTE FUNCTION refresh_models_for_tag();

CREATE OR REPLACE TRIGGER bundle_tags_rename_refresh
    AFTER UPDATE OF name ON tags
    FOR EACH ROW
    WHEN (OLD.name::text IS DISTINCT FROM NEW.name::text)
    EXECUTE FUNCTION refresh_bundles_for_tag();

CREATE OR REPLACE TRIGGER creators_rename_refresh
    AFTER UPDATE OF name ON creators
    FOR EACH ROW
    WHEN (OLD.name::text IS DISTINCT FROM NEW.name::text)
    EXECUTE FUNCTION refresh_models_for_creator();

CREATE OR REPLACE TRIGGER bundle_creators_rename_refresh
    AFTER UPDATE OF name ON creators
    FOR EACH ROW
    WHEN (OLD.name::text IS DISTINCT FROM NEW.name::text)
    EXECUTE FUNCTION refresh_bundles_for_creator();
