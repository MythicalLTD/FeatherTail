//! Builds the admin web UI (ui/) with npm so it can be embedded into the binary.
//!
//! - Set FEATHERTAIL_SKIP_UI_BUILD=1 to skip npm entirely (an existing ui/dist is used).
//! - If npm is missing and ui/dist does not exist, a placeholder page is generated so the
//!   Rust build never breaks; the UI then shows a "not built" message.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/index.html");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=ui/package-lock.json");
    println!("cargo:rerun-if-changed=ui/vite.config.ts");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");
    println!("cargo:rerun-if-env-changed=FEATHERTAIL_SKIP_UI_BUILD");

    let ui = Path::new("ui");
    let dist = ui.join("dist");

    if std::env::var("FEATHERTAIL_SKIP_UI_BUILD").is_ok() {
        ensure_placeholder(&dist);
        return;
    }

    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    if !ui.join("package.json").exists() || Command::new(npm).arg("--version").output().is_err() {
        println!("cargo:warning=npm not found: admin UI not rebuilt (using existing ui/dist or a placeholder)");
        ensure_placeholder(&dist);
        return;
    }

    if !ui.join("node_modules").exists() {
        let install = if ui.join("package-lock.json").exists() { "ci" } else { "install" };
        run(npm, &[install, "--no-audit", "--no-fund"], ui);
    }
    run(npm, &["run", "build"], ui);
}

fn run(cmd: &str, args: &[&str], dir: &Path) {
    let status = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {cmd} {args:?}: {e}"));
    if !status.success() {
        panic!("{cmd} {args:?} failed in {} ({status})", dir.display());
    }
}

fn ensure_placeholder(dist: &Path) {
    if dist.join("index.html").exists() {
        return;
    }
    std::fs::create_dir_all(dist).expect("create ui/dist");
    std::fs::write(
        dist.join("index.html"),
        "<!doctype html><meta charset=utf-8><title>FeatherTail</title><p style=\"font-family:sans-serif\">FeatherTail admin UI was not built into this binary. Install Node.js/npm and rebuild.</p>",
    )
    .expect("write placeholder ui");
}
