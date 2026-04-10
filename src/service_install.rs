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
    write_systemd_service()?;
    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", "--now", "feathertail.service"])?;

    Ok(())
}

fn ensure_root() -> Result<(), DynError> {
    let output = Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        return Err("failed to check user id".into());
    }

    let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if uid != "0" {
        return Err("service-install must be run as root".into());
    }

    Ok(())
}

fn install_binary() -> Result<(), DynError> {
    let source = std::env::current_exe()?;
    let target = Path::new(INSTALL_BIN_PATH);

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
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

fn write_systemd_service() -> Result<(), DynError> {
    let unit = format!(
        "[Unit]\nDescription=FeatherTail DHCP + Proxmox Agent\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={} --config {}\nRestart=on-failure\nRestartSec=3\nUser=root\nWorkingDirectory=/\n\n[Install]\nWantedBy=multi-user.target\n",
        INSTALL_BIN_PATH, INSTALL_CONFIG_PATH
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
