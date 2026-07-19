-- A carve can now commit only the files its rules matched ("keep unmatched
-- files staged"): the matched files move to the destination and the import
-- stays behind holding the rest, ready for another pass at a different target.
-- Such an import is flagged, so the Importing list can say some of it has
-- already been placed.
ALTER TABLE imports
    ADD COLUMN partial boolean NOT NULL DEFAULT false;
