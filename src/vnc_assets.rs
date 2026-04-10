use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

type DynError = Box<dyn Error + Send + Sync>;

const NOVNC_TARGET_DIR: &str = "/usr/share/novnc-pve";

const BUNDLED_FILES: &[(&str, &[u8])] = &[
    (
        "mgnovnc.html",
        include_bytes!("vnc/usr/share/novnc-pve/mgnovnc.html"),
    ),
    ("mgui.js", include_bytes!("vnc/usr/share/novnc-pve/mgui.js")),
    ("util.js", include_bytes!("vnc/usr/share/novnc-pve/util.js")),
    (
        "webutil.js",
        include_bytes!("vnc/usr/share/novnc-pve/webutil.js"),
    ),
];

pub fn is_proxmox_host() -> bool {
    let has_pve_dirs = Path::new("/etc/pve").is_dir()
        && (Path::new("/usr/bin/pvesh").exists() || Path::new("/bin/pvesh").exists());

    let has_pveversion = Command::new("pveversion")
        .arg("--verbose")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    has_pve_dirs || has_pveversion
}

pub fn install_bundled_vnc_assets() -> Result<(), DynError> {
    fs::create_dir_all(NOVNC_TARGET_DIR)?;

    for (name, content) in BUNDLED_FILES {
        let destination = Path::new(NOVNC_TARGET_DIR).join(name);
        fs::write(destination, content)?;
    }

    Ok(())
}
