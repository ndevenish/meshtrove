# MySTL

> **Historical** — the original project prompt, kept for provenance. The
> current design lives in `docs/plan.md`; decisions and status in
> `docs/decisions.md`. (The project became **MeshTrove**.)

A printables/Thingiverse like webapp to archive both downloaded/printed free
models, but also models that I have bought that I want to keep track of in one
central place, including notes on my printing settings and variants of models
that I have tweaked for printing. The app will be Rust/Axum/SQLx backend (with
migration) and React/Typescript/MaterialUI frontend structured according to
instructions in docs/PROJECT_TEMPLATE.md.

## Design Process

Please start by designing the database schema. I want to be able to track:

- Company/site/Author the model came from, as something distinctly trackable.
- Models. As a distinct "thing" but that can be comprised of many parts, with a
  kept folder structure. In addition, some models can have "variants" e.g. Loot
  Studios miniatures often come in 32mm, 75mm versions of the same model, and
  each of those come in variants Unsupported, Supported, Supported Hollow and
  as a direct Lychee project. I'll often want to make my own variant e.g. a
  model designed for resin might be split up but I personally went and merged
  them back into one model for printing, and would want that to be stored as
  it's own variant. The same model might also have Obj, STL versions, as well
  as other arbitrary binary representations. 3MF could be one of these, and
  could contain several model parts. Original file hashes should be kept, so
  that duplicates can be discovered based on exact STL hash.
- Associated model/Bundle documents e.g. the loot studios bundles are a lot of
  model variants, but have a "Bundle" associated magazine, stat guide, painting
  guide.
- Images. Models should be able to have associated images - these could come
  from a direct import/browser helper, but also the possibility to render an
  image from the STL directly, of configurable complexity e.g. it might be as
  complex as creating a blender scene and rendering that or as simple as some
  default rendering tool. The rendering of images (and potentially other
  processing) must be done in the background so there will need to be a way to
  keep track of outstanding jobs, be that rendering images or handling new
  imports/classifications/combining/compressing.
- Bundles of models e.g. Dragonlock have packs of terrain pieces that are
  specific purchaseable bundles, but it should also be possible to bundle all
  of these packs into an uber-bundle based on e.g. broad terrain categorisation.
- There should be a general model/bundle tagging system.
- Individual user login/registration. Users should be able to mark models as
  "liked", and mark any particular model as "printed". Depending on user role,
  they might be allowed to read-only (e.g. mark as wanted but not download),
  edit models/bundles they have created, or edit all (admin). This can be basic
  username/password hand-rolled for now.

Please make suggestions on ideas that are missing from this or problems that
this description introduces.

### Unresolved design issues

- Database should be PostGres. But I do not know the best way to (a) store
  images, and (b) store the raw model data - some sort of implicit filesystem
  hierarchy, or some sort of object store. If Object store, then I don't know
  the best way to split between buckets/objects, for both raw model data and
  image previews/other images/other associated documents.
- Whether the model tagging system should be overlapping with the variant
  system or whether they should remain completely separate. It might be useful
  to be able to request everything e.g. with a set of tags and variant like
  "miniature, egypt, undead, 32mm, unsupported".
