# Import layouts — real archive shapes

Reference for the file-first import + recategorisation feature. These are the
real archive layouts the import flow must cope with. One dropped archive can be
anything from a single model to a whole collection, and **you can't tell at drop
time** — the filename doesn't say and the contents aren't unpacked yet. So a
drop stages an **import**: the archive unpacks into a holding area that is
neither a model nor a bundle, and you place it only once you can see what's in
it. Layout detectors (Phase 3) then pre-fill that choice rather than making it.

See `docs/decisions.md` (status) and the phased plan for how this maps to code.

## A. Single model, a few flat files

`TinyCardinal.zip`:

```
Tiny Cardinal/
  Cinderwing3D Tiny Cardinal.stl                    # the raw model
  Cinderwing3D Tiny Cardinal Colors.3mf             # a print/colour config
  Cinderwing3D Tiny Cardinal Colors Blue Jay.3mf    # another print/colour config
```

**Target:** one model ("Tiny Cardinal") with ~3 variants — the raw STL, and the
two `.3mf` print/colour configurations. Shallow, no support-suffix folders.

## B. A whole collection (bundle of many models)

`DownloadAll_32mm.zip` (Loot Studios), deeply nested. The key trap: **top-level
folders are categories, not variants.**

```
DownloadAll_32mm/
  1 - Heroes/                                        # category (→ tag), NOT a variant
    BuriedTombHeroes_32mm_Supported_LYCHEE/          # set + scale + support-type
      Gold_32mm_Supported_Lychee/*.stl              # mini "Gold", support=lychee, scale=32mm
      Sanjay_32mm_Supported_Lychee/*.stl            # mini "Sanjay"
    BuriedTombHeroes_32mm_NoSupports/
      Gold_32mm_NoSupports/*.stl                    # same mini "Gold", support=unsupported
  2 - Enemies/   3 - NPC/   4 - Environment/   5 - Busts/   6 - Bonus/
```

Structure, decoded:

| Folder level | Example | Meaning |
|---|---|---|
| wrapper | `DownloadAll_32mm` | scale (`32mm`) for everything inside |
| top-level | `1 - Heroes` | **category** → model tag, NOT a variant |
| set + support | `BuriedTombHeroes_32mm_Supported_LYCHEE` | a set at a given support type |
| leaf | `Gold_32mm_Supported_Lychee` | one **mini** in one support variant |

The support-type suffixes seen in the wild: `_NoSupports`, `_Supported`,
`_Supported_LYCHEE`, `_Supported_Solid`, `_Supported_Hollow` — these map onto the
seeded variant tags (`unsupported`, `supported`, `lychee_project`, …).

**Target:** a **bundle** ("Buried Tomb"), containing many **models** (Gold,
Sanjay, Gynosphinx, …), each with **support-type variants**, all classified
**scale=32mm**. Dropping `DownloadAll_75mm.zip` later must **add into the same
bundle** (a model can span bundles — `bundle_models` is a true many-to-many).

## What the import flow does with these today

Both layouts already import and can be shaped by hand, because the destination
is chosen *after* the unpack:

- **A** — drop → the import page shows 3 files, one folder deep → commit as
  **one model** → move the `.3mf`s and the STL into variants on the model page.
- **B** — drop → commit as **a new bundle** (or *add to an existing bundle*, so
  `DownloadAll_75mm.zip` joins the same bundle later) → carve the bundle's
  unsorted files into member models, then each model's files into
  support-type variants.

What's missing is only that every step of B is manual: nothing reads
`_Supported_LYCHEE` or `32mm` out of the paths, and the destination toggle
always starts on *One model*.

## Heuristic for the model-vs-bundle suggestion (Phase 3)

Run over the **staged file tree** (the import is committed to nothing, so a
wrong guess costs an edit, never a conversion):

- **Looks like one model (A):** few files, shallow (≤1 folder deep), no
  support-suffix folders → preselect *One model*.
- **Looks like a bundle (B):** deep nesting, many leaf folders,
  `DownloadAll_<scale>` naming, support-type suffix folders → preselect *A new
  bundle*, and propose the carve: leaf folders → member models, support suffixes
  and the wrapper's `32mm` → variant tags on one variant, top-level `1 - Heroes`
  → a model tag rather than a variant tag.

Every one of these is an editable default on the import page; nothing commits
without the user pressing *Import*.
