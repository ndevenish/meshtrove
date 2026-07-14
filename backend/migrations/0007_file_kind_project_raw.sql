-- ---------------------------------------------------------------------------
-- Two kinds the 'model' bucket was hiding.
--
-- 'model' meant "geometry" but had quietly swallowed everything a printer or a
-- slicer touches. Split out what a file *is for*:
--
--   project — the editable source you reopen to change something: .3mf, .blend,
--             .lys, .lyt, .chitubox. Not geometry you slice; a document of work.
--   raw     — sliced machine output, cooked for one printer and one set of
--             settings: .ctb, .gcode. Not editable, not portable, not a model.
--
-- 'model' keeps pure geometry (.stl, .obj, .step, .stp, .ply, .gltf, .glb).
--
-- Adding the values is a migration of its own because Postgres will not let a
-- transaction *use* an enum value it added: the backfill lives in 0008.
-- ---------------------------------------------------------------------------

ALTER TYPE file_kind ADD VALUE IF NOT EXISTS 'project';
ALTER TYPE file_kind ADD VALUE IF NOT EXISTS 'raw';
