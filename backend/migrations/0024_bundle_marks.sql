-- Per-user marks on bundles.
--
-- A bundle is browsed as a peer of a model — one card in the same grid, taking
-- the place of the twenty models inside it — so whatever a user can say about a
-- model they must be able to say about a bundle, or "like everything on this
-- page" quietly skips half of it. `user_model_marks` cannot hold these: its FK
-- is to models. Same shape, same enum, different owner.

CREATE TABLE user_bundle_marks (
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    bundle_id  uuid NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    mark       mark_kind NOT NULL,
    notes      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, bundle_id, mark)
);
CREATE INDEX bundle_marks_by_bundle ON user_bundle_marks (bundle_id, mark);
