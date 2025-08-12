use anyhow::{bail, Context, Result};
use std::process::Command;

fn cmd_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).output().is_ok()
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd).args(args).status()
        .with_context(|| format!("failed to spawn {}", cmd))?;
    if !status.success() {
        bail!("command failed: {} {:?}", cmd, args);
    }
    Ok(())
}

pub fn ninja_exists() -> bool {
    cmd_ok("ninja", &["--version"])
}

pub fn ensure_ninja(auto_install: bool) -> Result<()> {
    if ninja_exists() {
        return Ok(());
    }
    if !auto_install {
        // Just tell the user what to do
        eprintln!("Ninja not found. Install it with one of:");
        if cfg!(windows) {
            eprintln!("  winget install Ninja-build.Ninja");
            eprintln!("  choco install ninja   # or: scoop install ninja");
        } else if cfg!(target_os = "macos") {
            eprintln!("  brew install ninja");
        } else {
            eprintln!("  sudo apt-get install -y ninja-build    # Debian/Ubuntu");
            eprintln!("  sudo dnf install -y ninja-build        # Fedora/RHEL");
            eprintln!("  sudo pacman -S --noconfirm ninja       # Arch");
            eprintln!("  sudo zypper install -y ninja           # openSUSE");
        }
        bail!("Ninja missing");
    }

    // Try to install automatically
    if cfg!(windows) {
        if cmd_ok("winget", &["--version"]) {
            return run("winget", &["install", "-e", "--id", "Ninja-build.Ninja"]);
        } else if cmd_ok("choco", &["--version"]) {
            return run("choco", &["install", "-y", "ninja"]);
        } else if cmd_ok("scoop", &["--version"]) {
            return run("scoop", &["install", "ninja"]);
        } else {
            bail!("No supported package manager found (winget/choco/scoop). Install Ninja manually.");
        }
    } else if cfg!(target_os = "macos") {
        if cmd_ok("brew", &["--version"]) {
            return run("brew", &["install", "ninja"]);
        } else {
            bail!("Homebrew not found. Install Homebrew or install Ninja manually.");
        }
    } else {
        // Linux
        if cmd_ok("apt-get", &["--version"]) {
            return run("sudo", &["apt-get", "update"])
                .and_then(|_| run("sudo", &["apt-get", "install", "-y", "ninja-build"]));
        } else if cmd_ok("dnf", &["--version"]) {
            return run("sudo", &["dnf", "install", "-y", "ninja-build"]);
        } else if cmd_ok("pacman", &["-V"]) {
            return run("sudo", &["pacman", "-S", "--noconfirm", "ninja"]);
        } else if cmd_ok("zypper", &["--version"]) {
            return run("sudo", &["zypper", "install", "-y", "ninja"]);
        } else {
            bail!("No supported package manager found. Install Ninja manually.");
        }
    }
}
