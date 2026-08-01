fn main() {
    let version = std::env::var("APKSULE_RELEASE_VERSION")
        .ok()
        .map(|value| value.trim().trim_start_matches('v').to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("GITHUB_REF_NAME").ok().and_then(|value| {
                let trimmed = value.trim();
                trimmed
                    .strip_prefix('v')
                    .map(str::to_owned)
                    .filter(|candidate| semver_like(candidate))
                    .or_else(|| if semver_like(trimmed) { Some(trimmed.to_owned()) } else { None })
            })
        })
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION"));

    println!("cargo:rerun-if-env-changed=APKSULE_RELEASE_VERSION");
    println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");
    println!("cargo:rustc-env=APKSULE_VERSION={version}");

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/apksule.ico");
        res.set("ProductName", "Apksule");
        res.set("FileDescription", "Apksule — lightweight APK compatibility runtime");
        res.set("LegalCopyright", "Copyright (c) 2026 Apksule contributors");
        res.set("ProductVersion", &version);
        res.set("FileVersion", &version);
        if let Err(error) = res.compile() {
            panic!("failed to embed Windows resources: {error}");
        }
    }
}

fn semver_like(value: &str) -> bool {
    let mut parts = value.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), Some(c), None)
            if a.chars().all(|ch| ch.is_ascii_digit())
                && b.chars().all(|ch| ch.is_ascii_digit())
                && c.chars().all(|ch| ch.is_ascii_digit())
    )
}
