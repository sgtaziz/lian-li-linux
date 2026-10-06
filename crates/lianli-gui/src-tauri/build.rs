#[path = "build/frontend.rs"]
mod frontend;

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    for input in frontend::INPUTS.iter().copied().chain(["public", "dist"]) {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    if let Err(error) = frontend::verify(&root) {
        let npm = find_npm().unwrap_or_else(|| {
            panic!("Frontend is not verified: {error}. Install npm and rebuild, or supply a frontend built from these sources with npm ci and npm run build.")
        });
        install_dependencies(&npm, &root);
        run(&npm, &["run", "build"], &root);
        frontend::verify(&root).expect("npm build did not produce a matching frontend manifest");
    }
    tauri_build::build();
}

const INSTALL_STAMP: &str = "node_modules/.lianli-npm-ci";

fn install_dependencies(npm: &Path, root: &Path) {
    let expected = dependency_fingerprint(root);
    let stamp = root.join(INSTALL_STAMP);
    if expected.is_some()
        && root.join("node_modules/.package-lock.json").is_file()
        && fs::read_to_string(&stamp).ok() == expected
    {
        return;
    }
    let _ = fs::remove_file(&stamp);
    run(npm, &["ci", "--no-audit", "--no-fund"], root);
    if let Some(fingerprint) = expected {
        fs::write(&stamp, fingerprint).expect("Failed to record npm ci stamp");
    }
}

fn dependency_fingerprint(root: &Path) -> Option<String> {
    let mut hasher = Sha256::new();
    for file in ["package.json", "package-lock.json"] {
        hasher.update(fs::read(root.join(file)).ok()?);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn find_npm() -> Option<PathBuf> {
    if Command::new("npm")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        return Some(PathBuf::from("npm"));
    }
    let output = Command::new("sh")
        .args(["-lc", "command -v npm"])
        .output()
        .ok()?;
    if output.status.success() {
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn run(command: &Path, args: &[&str], directory: &Path) {
    let status = Command::new(command)
        .args(args)
        .current_dir(directory)
        .status()
        .unwrap_or_else(|error| panic!("Failed to invoke {}: {error}", command.display()));
    assert!(
        status.success(),
        "{} {} exited with {status}",
        command.display(),
        args.join(" ")
    );
}
