// GearDot embeds its own Windows resources in the executable.
fn main() {
    // The app version lives in the VERSION file (Cargo versions cannot have four parts).
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let version = std::fs::read_to_string(root.join("VERSION")).unwrap().trim().to_string();
    println!("cargo:rerun-if-changed=VERSION");
    println!("cargo:rustc-env=BH_VERSION={version}");
    // Public half of the release-signing key (scripts/new-signing-key.ps1): when present, the
    // updater only starts setups whose SHA256SUMS.txt carries a valid signature from it.
    let key = root.join("release-signing.pub");
    println!("cargo:rerun-if-changed={}", key.display());
    if let Ok(hex) = std::fs::read_to_string(&key) {
        let hex = hex.trim();
        assert!(hex.len() == 128 && hex.chars().all(|c| c.is_ascii_hexdigit()), "release-signing.pub must hold 64 bytes (X || Y) as hex");
        println!("cargo:rustc-env=BH_RELEASE_KEY={hex}");
    }
}
