//! Versioning: VERSION holds MAJOR.MINOR; .build-number is incremented on every *release* build
//! (cargo build --release / cargo install), giving 1.0.001, 1.0.002, … Debug builds/tests reuse the
//! current number with a -dev suffix. Also embeds the short git sha.
use std::fs;
fn main() {
    let mm = fs::read_to_string("VERSION").unwrap_or_else(|_| "1.0".into()).trim().to_string();
    let mut n: u64 = fs::read_to_string(".build-number").ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    let release = std::env::var("PROFILE").map(|p| p == "release").unwrap_or(false);
    if release && std::env::var("HARNESS_NO_BUMP").is_err() { n += 1; let _ = fs::write(".build-number", format!("{n}\n")); }
    let sha = std::process::Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let version = format!("{mm}.{n:03}{}", if release { "" } else { "-dev" });
    println!("cargo:rustc-env=HARNESS_VERSION={version}");
    println!("cargo:rustc-env=HARNESS_BUILD={n}");
    println!("cargo:rustc-env=HARNESS_GIT={sha}");
    println!("cargo:rerun-if-changed=VERSION");
    println!("cargo:rerun-if-changed=.build-number");
    println!("cargo:rerun-if-env-changed=HARNESS_NO_BUMP");
    println!("cargo:rerun-if-env-changed=PROFILE");
}
