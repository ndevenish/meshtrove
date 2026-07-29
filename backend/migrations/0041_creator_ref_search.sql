-- `creator_ref` — the creator's own id/SKU for a model, added in 0026 — has been
-- editable on the model page and fillable from a carve layout ever since, but it
-- never reached the search vector: the one string a publisher's catalogue calls
-- the model by found nothing. Weight B, alongside the creator's name and the
-- tags: an id identifies the model as sharply as the creator does, but the
-- model's own name still outranks it.

CREATE OR REPLACE FUNCTION model_search_vector(model_id uuid) RETURNS tsvector AS $$
    SELECT setweight(to_tsvector('english', m.name), 'A')
         || setweight(to_tsvector('english', coalesce(c.name, '')), 'B')
         || setweight(to_tsvector('english', coalesce(m.creator_ref, '')), 'B')
         || setweight(to_tsvector('english', coalesce(
                (SELECT string_agg(t.name::text, ' ')
                   FROM model_tags mt JOIN tags t ON t.id = mt.tag_id
                  WHERE mt.model_id = m.id), '')), 'B')
         || setweight(to_tsvector('english', coalesce(
                (SELECT r.body_md FROM model_description_revisions r
                  WHERE r.model_id = m.id
                  ORDER BY r.created_at DESC LIMIT 1), '')), 'C')
      FROM models m LEFT JOIN creators c ON c.id = m.creator_id
     WHERE m.id = model_id
$$ LANGUAGE sql STABLE;

CREATE OR REPLACE FUNCTION models_search_before() RETURNS trigger AS $$
BEGIN
    NEW.search := setweight(to_tsvector('english', NEW.name), 'A')
        || setweight(to_tsvector('english', coalesce(
               (SELECT c.name FROM creators c WHERE c.id = NEW.creator_id), '')), 'B')
        || setweight(to_tsvector('english', coalesce(NEW.creator_ref, '')), 'B')
        || setweight(to_tsvector('english', coalesce(
               (SELECT string_agg(t.name::text, ' ')
                  FROM model_tags mt JOIN tags t ON t.id = mt.tag_id
                 WHERE mt.model_id = NEW.id), '')), 'B')
        || setweight(to_tsvector('english', coalesce(
               (SELECT r.body_md FROM model_description_revisions r
                 WHERE r.model_id = NEW.id
                 ORDER BY r.created_at DESC LIMIT 1), '')), 'C');
    RETURN NEW;
END
$$ LANGUAGE plpgsql;

-- Editing the id has to re-vectorize, so it joins the trigger's column list.
CREATE OR REPLACE TRIGGER models_search_refresh
    BEFORE INSERT OR UPDATE OF name, creator_id, creator_ref ON models
    FOR EACH ROW EXECUTE FUNCTION models_search_before();

-- Backfill the models that already carry one.
UPDATE models SET search = model_search_vector(id)
 WHERE creator_ref IS NOT NULL AND creator_ref <> '';
