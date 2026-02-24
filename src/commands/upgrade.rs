use std::env;
use std::fs;
use std::process::Command;

use anyhow::{bail, Context};

const REPO: &str = "polymarket/polymarket-cli";
const BINARY: &str = "polymarket";

pub fn execute() -> anyhow::Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: v{current_version}");
    println!("Checking for updates...");

    let latest_tag = get_latest_tag()?;
    let latest_version = latest_tag.trim_start_matches('v');

    if latest_version == current_version {
        println!("Already up to date.");
        return Ok(());
    }

    println!("New version available: {latest_tag}");

    let target = detect_target()?;
    let url = format!(
        "https://github.com/{REPO}/releases/download/{latest_tag}/{BINARY}-{latest_tag}-{target}.tar.gz"
    );

    let current_exe = env::current_exe().context("Failed to determine current executable path")?;

    let tmpdir = tempdir()?;
    let tarball = format!("{tmpdir}/{BINARY}.tar.gz");

    println!("Downloading {latest_tag} ({target})...");

    let status = Command::new("curl")
        .args(["-sSfL", "-o", &tarball, &url])
        .status()
        .context("Failed to run curl")?;
    if !status.success() {
        bail!("Download failed (HTTP error)");
    }

    let status = Command::new("tar")
        .args(["xzf", &tarball, "-C", &tmpdir])
        .status()
        .context("Failed to extract archive")?;
    if !status.success() {
        bail!("Failed to extract archive");
    }

    let new_binary = format!("{tmpdir}/{BINARY}");

    // Replace the current binary
    let exe_path = current_exe.to_str().context("Non-UTF8 executable path")?;
    let backup = format!("{exe_path}.bak");

    // Move current binary to backup, move new binary in, then remove backup
    fs::rename(exe_path, &backup)
        .or_else(|_| sudo_mv(exe_path, &backup))
        .context("Failed to replace binary (try running with sudo)")?;

    if let Err(e) = fs::rename(&new_binary, exe_path).or_else(|_| sudo_mv(&new_binary, exe_path))
    {
        // Restore backup on failure
        let _ = fs::rename(&backup, exe_path);
        return Err(e).context("Failed to install new binary");
    }

    // Set executable permission
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(exe_path, fs::Permissions::from_mode(0o755));
    }

    let _ = fs::remove_file(&backup);
    let _ = fs::remove_dir_all(&tmpdir);

    println!("Updated to {latest_tag}");
    Ok(())
}

fn get_latest_tag() -> anyhow::Result<String> {
    let output = Command::new("curl")
        .args([
            "-sSf",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .output()
        .context("Failed to check for latest release")?;

    if !output.status.success() {
        bail!("Failed to fetch latest release info from GitHub");
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse GitHub API response")?;

    json["tag_name"]
        .as_str()
        .map(String::from)
        .context("No tag_name in release response")
}

fn detect_target() -> anyhow::Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        _ => bail!("Unsupported platform: {os}/{arch}"),
    }
}

fn tempdir() -> anyhow::Result<String> {
    let output = Command::new("mktemp")
        .args(["-d"])
        .output()
        .context("Failed to create temp directory")?;
    if !output.status.success() {
        bail!("mktemp failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn sudo_mv(from: &str, to: &str) -> std::io::Result<()> {
    let status = Command::new("sudo")
        .args(["mv", from, to])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "sudo mv failed",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_target_returns_valid_triple() {
        let target = detect_target().unwrap();
        assert!(
            target.contains("apple-darwin") || target.contains("unknown-linux"),
            "unexpected target: {target}"
        );
    }
}
