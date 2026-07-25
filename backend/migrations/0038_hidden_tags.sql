-- Admin-hideable tags for content filtering (e.g. an "nsfw" tag). A tag flagged
-- hidden hides every model and bundle that carries it from the browse grid and
-- from search — the items are excluded at query time (see push_*_hidden_exclude
-- in the browse/tags routes), not deleted or altered. Only an admin can reveal
-- them, via the "Show hidden" toggle that threads `show_hidden` into the browse
-- and tag-list queries. Nothing about a tag's associations changes, so hiding is
-- fully reversible: unhide and the items return exactly as they were.
ALTER TABLE tags ADD COLUMN hidden boolean NOT NULL DEFAULT false;
