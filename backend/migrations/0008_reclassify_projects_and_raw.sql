-- ---------------------------------------------------------------------------
-- Reclassify what is already stored (see 0007 for the split).
--
-- Only files still carrying the kind the *guess* gave them are touched: 'model'
-- (what .3mf/.lys/.lyt/.chitubox/.ctb landed as) and 'other' (.blend/.gcode).
-- A file someone deliberately re-kinded to 'document' or 'archive' said
-- something the extension does not know, and keeps it.
-- ---------------------------------------------------------------------------

UPDATE files
SET kind = 'project'
WHERE kind IN ('model', 'other')
  AND lower(filename) ~ '\.(3mf|blend|lys|lyt|chitubox)$';

UPDATE files
SET kind = 'raw'
WHERE kind IN ('model', 'other')
  AND lower(filename) ~ '\.(ctb|gcode)$';
