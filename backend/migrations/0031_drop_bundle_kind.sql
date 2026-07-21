-- Drop the bundle `kind` (purchased | collection).
--
-- It was a two-value enum baked into the schema, the API and two edit forms,
-- and it never earned that: nothing branched on it — no filter, no query, no
-- rule — it only rendered as a chip on the bundle page. That is exactly what
-- the admin-defined custom fields (0030) are for, so it comes back as one,
-- editable and extensible, instead of a column and a migration per value.
--
-- The values are not preserved. Everything defaulted to 'purchased' unless
-- someone changed it by hand, and the replacement field starts empty on
-- purpose: a bundle says what it is once someone says so.

ALTER TABLE bundles DROP COLUMN kind;
DROP TYPE bundle_kind;
