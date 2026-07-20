use std::process::Command;

/// `git describe` renders "52 commits past tag v1.1" as `v1.1-52-gebc55f2`.
/// Fold the count in as a version component instead — `v1.1.52-gebc55f2` — so
/// the leading part reads as one version rather than a tag with a suffix bolted
/// on.
///
/// Anything not of that shape passes through untouched: a build sitting exactly
/// on a tag (`v1.1`), an untagged repo's bare sha (`ebc55f2`), or a version a
/// build environment injected verbatim.
///
/// Mirrored in frontend/vite.config.ts, which stamps the SPA the same way — the
/// two are compared for equality to detect a redeploy, so they must agree.
fn normalize(describe: &str) -> String {
    // Split off the trailing `-g<sha>` (kept as-is, `-dirty` and all), then peel
    // the commit count off the end of what precedes it.
    let Some(sha_at) = describe.rfind("-g") else {
        return describe.to_string();
    };
    let head = &describe[..sha_at];
    let Some(dash) = head.rfind('-') else {
        return describe.to_string();
    };
    let count = &head[dash + 1..];
    if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
        return describe.to_string();
    }
    format!("{}.{}{}", &head[..dash], count, &describe[sha_at..])
}

fn main() {
    // A build environment (e.g. the Docker image build) with no .git can inject
    // the version directly; otherwise derive it from git, falling back to unknown.
    let version = std::env::var("APP_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| {
            Command::new("git")
                .args(["describe", "--tags", "--always", "--dirty"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        });
    println!("cargo:rustc-env=APP_VERSION={}", normalize(&version));
    println!("cargo:rerun-if-env-changed=APP_VERSION");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs");
}
