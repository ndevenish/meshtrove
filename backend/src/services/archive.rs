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
/// [`crate::services::importer`]). Falls back to dropping the last extension so
/// an archive we don't recognize still names a folder something sane.
pub fn stem_of(filename: &str) -> &str {
    match suffix_of(filename) {
        Some((suffix, _)) => &filename[..filename.len() - suffix.len()],
        None => filename.rsplit_once('.').map_or(filename, |(stem, _)| stem),
    }
}

#[cfg(test)]
mod tests {
    use super::{Format, format_of, stem_of};

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
}
