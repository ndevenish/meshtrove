/// URL-safe slug: lowercase alphanumerics with single dashes.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_dash = true;
    for c in name.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "item".to_string()
    } else {
        slug
    }
}

/// Put the spaces back into a name that never had them: `KnightRider` reads as
/// "Knight Rider", `STLKnight` as "STL Knight". Archives name folders in camel
/// case constantly, and a library full of `DwarfBerserkerAxe` is a library you
/// cannot skim.
///
/// Two boundaries, and only two: lower/digit followed by upper (`rKnight`), and
/// a run of capitals followed by a capitalised word (`STLKnight` — the run is an
/// acronym, and the last letter of it starts the next word). Casing is left
/// exactly as found: an acronym stays an acronym.
pub fn expand_camel_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let ends_a_word = prev.is_lowercase() || prev.is_numeric();
            let ends_an_acronym = prev.is_uppercase() && next_is_lower;
            if (ends_a_word || ends_an_acronym) && !out.ends_with(' ') {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// Make a captured folder/file token readable before we show it back to the
/// user to map onto tags: underscores become spaces, camelCase gets its spaces
/// back, and runs of whitespace collapse to one. `Supported_PreLychee` reads as
/// "Supported Pre Lychee". Purely cosmetic — the value's identity (its folded
/// form, the value-map key) is untouched, so mapping and matching still key off
/// the raw capture.
pub fn humanize_token(raw: &str) -> String {
    let expanded = expand_camel_case(&raw.replace('_', " "));
    expanded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{expand_camel_case, humanize_token, slugify};

    #[test]
    fn camel_case_gets_its_spaces_back() {
        assert_eq!(expand_camel_case("KnightRider"), "Knight Rider");
        assert_eq!(
            expand_camel_case("DwarfBerserkerAxe"),
            "Dwarf Berserker Axe"
        );
        // An acronym is a word: the run breaks before the capital that starts
        // the next one, not between every letter of it.
        assert_eq!(expand_camel_case("STLKnight"), "STL Knight");
        assert_eq!(expand_camel_case("USB"), "USB");
        // Digits end a word too.
        assert_eq!(expand_camel_case("Knight2Pose"), "Knight2 Pose");
        // Already spaced, or nothing to do: left alone.
        assert_eq!(expand_camel_case("Knight Rider"), "Knight Rider");
        assert_eq!(expand_camel_case("knight"), "knight");
        assert_eq!(expand_camel_case(""), "");
    }

    #[test]
    fn tokens_get_humanised_for_display() {
        assert_eq!(humanize_token("Supported_LYCHEE"), "Supported LYCHEE");
        assert_eq!(humanize_token("PreSupported"), "Pre Supported");
        assert_eq!(humanize_token("32_mm"), "32 mm");
        // Underscores and camelCase together, and doubled/edge whitespace.
        assert_eq!(
            humanize_token("Supported_PreLychee"),
            "Supported Pre Lychee"
        );
        assert_eq!(humanize_token("  spare___parts_kit "), "spare parts kit");
        assert_eq!(humanize_token(""), "");
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Anubis Warrior"), "anubis-warrior");
        assert_eq!(slugify("  32mm / Supported!  "), "32mm-supported");
        assert_eq!(slugify("Ünïcode Náme"), "ünïcode-náme");
        assert_eq!(slugify("!!!"), "item");
    }
}
