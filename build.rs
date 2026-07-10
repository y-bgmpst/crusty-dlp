use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=plugins");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    if let Some(reference) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git_output(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    let git_sha =
        git_output(&["rev-parse", "--short=10", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let git_dirty = git_dirty_state();
    let build_timestamp = build_timestamp().unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=CRUSTY_GIT_SHA={git_sha}");
    println!("cargo:rustc-env=CRUSTY_GIT_DIRTY={git_dirty}");
    println!("cargo:rustc-env=CRUSTY_BUILD_TIMESTAMP={build_timestamp}");
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn git_dirty_state() -> &'static str {
    match Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
    {
        Ok(output) if output.status.success() && output.stdout.is_empty() => "clean",
        Ok(output) if output.status.success() => "dirty",
        _ => "unknown",
    }
}

fn build_timestamp() -> Option<String> {
    if let Ok(value) = std::env::var("SOURCE_DATE_EPOCH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }

    command_text("date", &["-u", "+%Y-%m-%d %H:%M:%SZ"]).or_else(|| {
        command_text(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "Get-Date -Format 'yyyy-MM-dd HH:mm:ssZ'",
            ],
        )
    })
}

fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
