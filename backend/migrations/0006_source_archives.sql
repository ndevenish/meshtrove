-- ---------------------------------------------------------------------------
-- source_archives: what an import was unpacked *from*, after the bytes are gone.
--
-- Keeping the original zip as a file row (decisions.md: "for provenance") meant
-- the store held the archive *and* every file inside it — a permanent surcharge
-- of ~1.3-1.5x on top of the extracted copy, for bytes nobody ever browses.
-- Committing an import now deletes the archive and leaves this row in its place:
-- the name, hash and size of what was dropped, which is the provenance anyone
-- actually asks for ("where did this model come from?"), at ~100 bytes instead
-- of gigabytes. The sha256 is *not* a blobs FK: the blob is deliberately gone.
-- ---------------------------------------------------------------------------

CREATE TABLE source_archives (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id    uuid REFERENCES models (id) ON DELETE CASCADE,
    bundle_id   uuid REFERENCES bundles (id) ON DELETE CASCADE,
    filename    text NOT NULL,
    sha256      char(64) NOT NULL,
    size        bigint NOT NULL,
    imported_at timestamptz NOT NULL DEFAULT now(),
    CHECK (num_nonnulls(model_id, bundle_id) = 1)
);
CREATE INDEX source_archives_by_model ON source_archives (model_id) WHERE model_id IS NOT NULL;
CREATE INDEX source_archives_by_bundle ON source_archives (bundle_id) WHERE bundle_id IS NOT NULL;
