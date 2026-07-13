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

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Anubis Warrior"), "anubis-warrior");
        assert_eq!(slugify("  32mm / Supported!  "), "32mm-supported");
        assert_eq!(slugify("Ünïcode Náme"), "ünïcode-náme");
        assert_eq!(slugify("!!!"), "item");
    }
}
