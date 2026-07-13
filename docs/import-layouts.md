# Import layouts — real archive shapes

Reference for the file-first import + recategorisation feature. These are the
real archive layouts the import flow must cope with. One dropped archive can be
anything from a single model to a whole collection, and **you usually can't tell
at drop time** — which is why the flow is *import flat, then recategorise*, with
layout-specific detectors added later (Phase 3) rather than guessed up front.

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
seeded `support` axis (`unsupported`, `supported`, `lychee_project`, …).

**Target:** a **bundle** ("Buried Tomb"), containing many **models** (Gold,
Sanjay, Gynosphinx, …), each with **support-type variants**, all classified
**scale=32mm**. Dropping `DownloadAll_75mm.zip` later must **add into the same
bundle** (a model can span bundles — `bundle_models` is a true many-to-many).

## Heuristic for the model-vs-bundle guess (Phase 3)

- **Looks like one model (A):** few files, shallow (≤1 folder deep), no
  support-suffix folders.
- **Looks like a bundle (B):** deep nesting, many leaf folders,
  `DownloadAll_<scale>` naming, support-type suffix folders.

The guess is only ever an editable default; the recategorisation UI can always
re-shape model↔bundle by hand.

## What Phase 1 actually does with these

Phase 1 imports **flat into one model's "unsorted" bucket** and provides the
recategorisation UI (set kind, move files into variants, delete, rename).
- **A** is handled fully: drop → model → move the `.3mf`s and STL into variants.
- **B** works manually: everything lands in one model you can tidy; proper
  bundle-of-models splitting is Phase 2, and auto-detection of the layout above
  is Phase 3.
