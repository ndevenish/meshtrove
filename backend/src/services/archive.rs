//! What counts as an archive, and how its name comes apart.
//!
//! One table, three consumers: the kind heuristic that labels a dropped file
//! ([`crate::routes::files::guess_kind`]), the ingest gate that decides whether
//! to queue an unpack ([`crate::routes::files::on_archive_ingested`]), and the
//! unpacker itself, which picks a reader per format. They have to agree — a
//! file labelled `archive` that the gate skips sits in an import looking dealt
//! with when nothing ever opened it.

/// Which reader opens this archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Read in-process by the `zip` crate.
    Zip,
    /// Shelled out to libarchive via `bsdtar`: tar and its compressed forms,
    /// 7z, and rar. One tool for all of them — libarchive reads rar5 and 7z
    /// with its own implementation, so no non-free unrar source rides along.
    Libarchive,
}

/// Recognized suffixes, **longest first**: `.tar.gz` has to win over `.tgz`'s
/// neighbours and over a bare-extension fallback, or the stem keeps its `.tar`
/// and the unpack lands in a folder called `dragon.tar`.
///
/// Bare `.gz`/`.xz`/`.bz2` are deliberately absent. They are a single
/// compressed file, not an archive, and libarchive needs `--format raw` to make
/// anything of them — claiming them here would queue an unpack that always
/// fails.
const SUFFIXES: &[(&str, Format)] = &[
    (".tar.gz", Format::Libarchive),
    (".tar.bz2", Format::Libarchive),
    (".tar.zst", Format::Libarchive),
    (".tar.xz", Format::Libarchive),
    (".tbz2", Format::Libarchive),
    (".tzst", Format::Libarchive),
    (".tgz", Format::Libarchive),
    (".tbz", Format::Libarchive),
    (".txz", Format::Libarchive),
    (".tar", Format::Libarchive),
    (".zip", Format::Zip),
    (".rar", Format::Libarchive),
    (".7z", Format::Libarchive),
];

/// The recognized suffix on `filename`, matched case-insensitively.
fn suffix_of(filename: &str) -> Option<(&'static str, Format)> {
    let lower = filename.to_lowercase();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix) && lower.len() > suffix.len())
        .copied()
}

/// Which reader opens `filename`, or `None` if it is not an archive we unpack.
pub fn format_of(filename: &str) -> Option<Format> {
    suffix_of(filename).map(|(_, format)| format)
}

/// The name with its archive suffix removed — what the unpack folder is called
/// when the archive has to make room for a sibling (see
/// [`crate::services::importer`]). A volume gives up its volume number too, so a
/// set is named after the set. Falls back to dropping the last extension so an
/// archive we don't recognize still names a folder something sane.
pub fn stem_of(filename: &str) -> &str {
    if let Some(volume) = volume_of(filename) {
        return volume.set;
    }
    match suffix_of(filename) {
        Some((suffix, _)) => &filename[..filename.len() - suffix.len()],
        None => filename.rsplit_once('.').map_or(filename, |(stem, _)| stem),
    }
}

/// How a set spells its volumes. The two styles never mix inside one set, and
/// legacy volume 1 is spelled exactly like a rar that has no volumes at all —
/// so a set only ever gathers volumes of its own style, or `Dragon.rar` and
/// `Dragon.part1.rar` sitting in one folder would be read as one set with two
/// first volumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeStyle {
    /// rar3 and later: `Dragon.part1.rar`, `Dragon.part2.rar`.
    Part,
    /// rar2-era: `Dragon.rar`, then `Dragon.r00`, `Dragon.r01`.
    Legacy,
}

/// One volume of a rar set — which set it belongs to, and where in it this
/// volume falls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume<'a> {
    /// What the set is called: `Dragon` for `Dragon.part2.rar`.
    pub set: &'a str,
    /// 1-based position. Volume 1 is the only one anything is ever pointed at;
    /// libarchive walks to the rest by name from there, which is why they have
    /// to sit in one directory (see [`crate::services::importer`]).
    pub index: u32,
    pub style: VolumeStyle,
}

/// Read `filename` as a rar volume, or `None` if it isn't one.
///
/// A plain `Dragon.rar` reads as volume 1 of a legacy set — from the name alone
/// there is no telling it apart from the first volume of one, and a set of one
/// behaves exactly like the lone archive it is.
///
/// `.rNN` continuation volumes are deliberately **not** in [`SUFFIXES`]: nothing
/// can open one on its own, so labelling it an archive would queue an unpack
/// that always fails. They are only ever gathered up beside their `.rar`.
pub fn volume_of(filename: &str) -> Option<Volume<'_>> {
    let lower = filename.to_lowercase();
    if let Some(head) = lower.strip_suffix(".rar") {
        if let Some(dot) = head.rfind(".part") {
            let digits = &head[dot + ".part".len()..];
            let index = digits.parse::<u32>().ok().filter(|i| *i >= 1);
            if let (Some(index), Some(set)) = (index, named(&filename[..dot])) {
                return Some(Volume {
                    set,
                    index,
                    style: VolumeStyle::Part,
                });
            }
        }
        return Some(Volume {
            set: named(&filename[..head.len()])?,
            index: 1,
            style: VolumeStyle::Legacy,
        });
    }
    // `.r00` is the *second* volume of its set; the `.rar` is the first.
    let (head, ext) = lower.rsplit_once('.')?;
    let digits = ext.strip_prefix('r').filter(|d| d.len() == 2)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(Volume {
        set: named(&filename[..head.len()])?,
        index: digits.parse::<u32>().ok()? + 2,
        style: VolumeStyle::Legacy,
    })
}

/// The set name, if there is one to speak of. `.rar` and `.part1.rar` are
/// hidden files called "rar" and "part1.rar" — a volume of nothing, and a name
/// no folder should ever be called after.
fn named(set: &str) -> Option<&str> {
    (!set.is_empty() && !set.starts_with('.')).then_some(set)
}

/// Is this the volume an unpack is pointed at? True for anything that is not a
/// volume at all, so every other format flows through the callers unchanged.
pub fn is_first_volume(filename: &str) -> bool {
    volume_of(filename).is_none_or(|volume| volume.index == 1)
}

/// Are these two names volumes of one set? Case-insensitively, since the set
/// name is part of a filename and filenames arrive spelled however they were
/// spelled.
pub fn same_volume_set(a: &str, b: &str) -> bool {
    match (volume_of(a), volume_of(b)) {
        (Some(a), Some(b)) => a.style == b.style && a.set.eq_ignore_ascii_case(b.set),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Format, VolumeStyle as Style, format_of, is_first_volume, same_volume_set, stem_of,
        volume_of,
    };

    #[test]
    fn the_tar_family_is_an_archive_extension_too() {
        // The reported hole: `.tar.gz` split on the last dot alone read as a
        // `gz` file of no known kind, so nothing ever tried to open it.
        for name in [
            "pack.tar.gz",
            "pack.tar.bz2",
            "pack.tar.xz",
            "pack.tar.zst",
            "pack.tgz",
            "pack.txz",
            "pack.tar",
            "pack.rar",
            "pack.7z",
        ] {
            assert_eq!(format_of(name), Some(Format::Libarchive), "{name}");
        }
        assert_eq!(format_of("pack.zip"), Some(Format::Zip));
    }

    #[test]
    fn extensions_match_whatever_case_they_arrive_in() {
        assert_eq!(format_of("PACK.RAR"), Some(Format::Libarchive));
        assert_eq!(format_of("Pack.Tar.Gz"), Some(Format::Libarchive));
        assert_eq!(stem_of("Pack.Tar.Gz"), "Pack");
    }

    #[test]
    fn a_name_that_is_only_an_extension_is_not_an_archive() {
        // `.zip` is a hidden file called "zip", not an archive named nothing.
        assert_eq!(format_of(".zip"), None);
        assert_eq!(format_of(".tar.gz"), None);
    }

    #[test]
    fn single_file_compression_is_not_an_archive() {
        // libarchive needs --format raw for these; queuing an unpack would
        // only ever fail.
        assert_eq!(format_of("dragon.stl.gz"), None);
        assert_eq!(format_of("notes.xz"), None);
    }

    #[test]
    fn the_whole_suffix_comes_off_the_stem() {
        assert_eq!(stem_of("dragon.tar.gz"), "dragon");
        assert_eq!(stem_of("dragon.v2.zip"), "dragon.v2");
        assert_eq!(stem_of("dragon.7z"), "dragon");
        // Unrecognized: last extension only, so a folder still gets a name.
        assert_eq!(stem_of("dragon.arj"), "dragon");
        assert_eq!(stem_of("dragon"), "dragon");
    }

    #[test]
    fn a_volume_number_is_not_part_of_the_set_name() {
        // The set is one archive in several files: it unpacks into one folder,
        // named after the set — not `Dragon.part1`, and not three folders.
        assert_eq!(stem_of("Dragon.part1.rar"), "Dragon");
        assert_eq!(stem_of("Dragon.part12.rar"), "Dragon");
        assert_eq!(stem_of("Dragon.r00"), "Dragon");
        assert_eq!(stem_of("Dragon.rar"), "Dragon");
    }

    #[test]
    fn volumes_number_themselves_from_one() {
        let part = volume_of("Dragon.part03.rar").unwrap();
        assert_eq!(
            (part.set, part.index, part.style),
            ("Dragon", 3, Style::Part)
        );
        // The `.rar` is volume 1 of a legacy set, so `.r00` is the second.
        assert_eq!(volume_of("Dragon.rar").unwrap().index, 1);
        assert_eq!(volume_of("Dragon.r00").unwrap().index, 2);
        assert_eq!(volume_of("Dragon.r01").unwrap().index, 3);
        assert!(volume_of("Dragon.rar").unwrap().style == Style::Legacy);
    }

    #[test]
    fn only_the_first_volume_is_ever_opened() {
        // What the ingest gate turns on: point libarchive at volume 1 and it
        // walks to the rest itself; point it at volume 2 and it sees a
        // truncated archive.
        assert!(is_first_volume("Dragon.part1.rar"));
        assert!(is_first_volume("Dragon.part01.rar"));
        assert!(is_first_volume("Dragon.rar"));
        assert!(!is_first_volume("Dragon.part2.rar"));
        assert!(!is_first_volume("Dragon.r00"));
        // Everything that is not a volume is its own whole archive.
        assert!(is_first_volume("Dragon.zip"));
        assert!(is_first_volume("Dragon.tar.gz"));
    }

    #[test]
    fn a_set_gathers_its_own_volumes_only() {
        assert!(same_volume_set("Dragon.part1.rar", "Dragon.part2.rar"));
        assert!(same_volume_set("Dragon.part1.rar", "DRAGON.PART2.RAR"));
        assert!(same_volume_set("Dragon.rar", "Dragon.r00"));
        // Two packs that happen to share a folder are two archives.
        assert!(!same_volume_set("Dragon.part1.rar", "Griffin.part2.rar"));
        // Same name, different spelling of "volume": `Dragon.rar` is not the
        // first volume of the `Dragon.partN.rar` set, it is its own archive.
        assert!(!same_volume_set("Dragon.rar", "Dragon.part2.rar"));
        // A zip is never a volume of anything.
        assert!(!same_volume_set("Dragon.zip", "Dragon.rar"));
    }

    #[test]
    fn a_name_that_only_looks_like_a_volume_is_not_one() {
        // A hidden file called "rar", and a `.part` with no set in front of it.
        assert_eq!(volume_of(".rar"), None);
        assert_eq!(volume_of(".part1.rar"), None);
        // `.r00` needs exactly two digits and something to belong to.
        assert_eq!(volume_of("dragon.rev"), None);
        assert_eq!(volume_of("dragon.r0"), None);
        assert_eq!(volume_of("dragon.r000"), None);
        assert_eq!(volume_of(".r00"), None);
        // `.partN` is only a volume number on a rar.
        assert_eq!(volume_of("dragon.part1.zip"), None);
        // …and a rar whose stem merely ends in something part-like isn't one.
        assert_eq!(volume_of("dragon.parts.rar").unwrap().set, "dragon.parts");
    }
}
