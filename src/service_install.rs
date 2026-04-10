use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::AppConfig;
use crate::vnc_assets;

type DynError = Box<dyn Error + Send + Sync>;

const INSTALL_BIN_PATH: &str = "/usr/local/bin/feathertail";
const INSTALL_CONFIG_PATH: &str = "/etc/feathertail/feathertail.toml";
const INSTALL_CONFIG_DIR: &str = "/etc/feathertail";
const SERVICE_PATH: &str = "/etc/systemd/system/feathertail.service";

pub fn install_service(source_config_path: &str) -> Result<(), DynError> {
    ensure_root()?;

    if !vnc_assets::is_proxmox_host() {
        return Err("FeatherTail agent only runs on proxmox hosts".into());
    }

    install_binary()?;
    install_config(source_config_path)?;
    write_systemd_service(INSTALL_BIN_PATH, INSTALL_CONFIG_PATH)?;
    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", "--now", "feathertail.service"])?;

    Ok(())
}

pub fn refresh_managed_service_on_start(config_path: &str) -> Result<(), DynError> {
    if !is_root()? {
        return Ok(());
    }

    let current_exe = std::env::current_exe()?;
    if current_exe != Path::new(INSTALL_BIN_PATH) {
        return Ok(());
    }

    if !Path::new(SERVICE_PATH).exists() {
        return Ok(());
    }

    write_systemd_service(INSTALL_BIN_PATH, config_path)?;
    run_systemctl(&["daemon-reload"])?;
    Ok(())
}

fn ensure_root() -> Result<(), DynError> {
    if is_root()? {
        return Ok(());
    }

    Err("service-install must be run as root".into())
}

fn is_root() -> Result<bool, DynError> {
    let output = Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        return Err("failed to check user id".into());
    }

    let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(uid == "0")
}

fn install_binary() -> Result<(), DynError> {
    let source = std::env::current_exe()?;
    let target = Path::new(INSTALL_BIN_PATH);

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    // If target exists and service is running, stop it first to avoid "text file busy" error
    if target.exists() {
        let service_running = Command::new("systemctl")
            .args(&["is-active", "--quiet", "feathertail.service"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if service_running {
            run_systemctl(&["stop", "feathertail.service"])?;
        }
    }

    fs::copy(&source, target)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(target)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(target, perms)?;
    }

    Ok(())
}

fn install_config(source_config_path: &str) -> Result<(), DynError> {
    fs::create_dir_all(INSTALL_CONFIG_DIR)?;

    let source = Path::new(source_config_path);
    if source.exists() {
        fs::copy(source, INSTALL_CONFIG_PATH)?;
    } else {
        let default = AppConfig::default();
        let serialized = toml::to_string_pretty(&default)?;
        fs::write(INSTALL_CONFIG_PATH, serialized)?;
    }

    Ok(())
}

fn write_systemd_service(exec_path: &str, config_path: &str) -> Result<(), DynError> {
    let unit = format!(
        "[Unit]\nDescription=FeatherTail DHCP + Proxmox Agent\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={} --config {}\nRestart=on-failure\nRestartSec=2\nTimeoutStartSec=20\nTimeoutStopSec=12\nKillMode=mixed\nUser=root\nWorkingDirectory=/\n\n[Install]\nWantedBy=multi-user.target\n",
        exec_path, config_path
    );

    fs::write(SERVICE_PATH, unit)?;
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<(), DynError> {
    let output = Command::new("systemctl").args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!("systemctl {} failed: {}", args.join(" "), stderr).into());
    }

    Ok(())
}
