//! `feathertail configure`: interactive setup wizard.
//!
//! Two modes: FeatherPanel (panel-issued API token) and simple standalone DHCP
//! (`[[dhcp.ranges]]`, no FeatherPanel needed). Both can be combined. The config file is
//! edited in place with `toml_edit`, so comments and unrelated keys are kept.

use std::io::{self, BufRead, Write};
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::Command;

use crate::config::{AppConfig, DhcpRange};

const PLACEHOLDER_TOKEN: &str = "change-me";

pub fn run(config_path: &str) -> Result<(), String> {
    if !Path::new(config_path).exists() {
        AppConfig::load_or_bootstrap(Path::new(config_path)).map_err(|e| e.to_string())?;
    }
    let contents = std::fs::read_to_string(config_path)
        .map_err(|e| format!("failed to read {config_path}: {e}"))?;
    let current: AppConfig =
        toml::from_str(&contents).map_err(|e| format!("failed to parse {config_path}: {e}"))?;
    let mut doc: toml_edit::DocumentMut = contents
        .parse()
        .map_err(|e| format!("failed to parse {config_path}: {e}"))?;

    println!();
    println!("FeatherTail setup ({config_path})");
    println!("Press Enter to keep the value in [brackets].");
    println!();
    println!("How will this node be used?");
    println!("  [1] With FeatherPanel");
    println!("  [2] Simple DHCP server only (no FeatherPanel)");
    println!("  [3] Both: FeatherPanel + simple DHCP for other clients");
    let mode = loop {
        match ask("Choice", "2")?.as_str() {
            "1" => break 1,
            "2" => break 2,
            "3" => break 3,
            _ => println!("  please type 1, 2 or 3"),
        }
    };
    let with_panel = mode != 2;
    let with_ranges = mode != 1;

    // ---- API / auth -------------------------------------------------------------------
    let existing_token = current.auth.api_token.trim().to_owned();
    let token_is_placeholder = existing_token.is_empty() || existing_token == PLACEHOLDER_TOKEN;
    let mut generated_token = false;
    let token = if with_panel {
        let default = std::env::var("FP_TAIL_TOKEN")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| if token_is_placeholder { String::new() } else { existing_token.clone() });
        loop {
            let t = ask("FeatherPanel API token (from the node's FeatherTail tab)", &default)?;
            if t.is_empty() || t == PLACEHOLDER_TOKEN {
                println!("  a real token is required for FeatherPanel");
                continue;
            }
            break t;
        }
    } else if token_is_placeholder {
        generated_token = true;
        random_token()?
    } else {
        existing_token.clone()
    };
    let api_bind = loop {
        let default = if current.api.bind.is_empty() { "0.0.0.0:8686" } else { &current.api.bind };
        let v = ask("API / web UI listen address", default)?;
        if v.parse::<std::net::SocketAddr>().is_ok() {
            break v;
        }
        println!("  use ip:port, e.g. 0.0.0.0:8686");
    };

    // ---- DHCP -------------------------------------------------------------------------
    let dhcp_enabled = ask_yes("Enable the DHCP server?", with_ranges || current.dhcp.enabled)?;
    let lease_time = loop {
        let v = ask("Default lease time (seconds)", &current.dhcp.lease_time_secs.to_string())?;
        match v.parse::<u64>() {
            Ok(n) if n >= 60 => break n,
            _ => println!("  enter a number >= 60"),
        }
    };

    let mut ranges: Vec<DhcpRange> = Vec::new();
    let mut replace_ranges = false;
    if with_ranges {
        if !current.dhcp.ranges.is_empty() {
            println!();
            println!("Existing DHCP ranges:");
            for r in &current.dhcp.ranges {
                println!("  {} {}-{} via {}/{}", iface_label(&r.interface), r.start, r.end, r.gateway, r.cidr);
            }
            replace_ranges = !ask_yes("Keep these ranges?", true)?;
        } else {
            replace_ranges = true;
        }
        if replace_ranges {
            loop {
                println!();
                ranges.push(ask_range()?);
                if !ask_yes("Add another range (another bridge/subnet)?", false)? {
                    break;
                }
            }
        }
    }

    // ---- Write ------------------------------------------------------------------------
    if !doc.contains_table("api") {
        doc["api"] = toml_edit::table();
    }
    if !doc.contains_table("auth") {
        doc["auth"] = toml_edit::table();
    }
    if !doc.contains_table("dhcp") {
        doc["dhcp"] = toml_edit::table();
    }
    doc["api"]["bind"] = toml_edit::value(api_bind.clone());
    doc["auth"]["api_token"] = toml_edit::value(token.clone());
    doc["dhcp"]["enabled"] = toml_edit::value(dhcp_enabled);
    doc["dhcp"]["lease_time_secs"] = toml_edit::value(i64::try_from(lease_time).unwrap_or(86_400));
    if replace_ranges {
        let mut aot = toml_edit::ArrayOfTables::new();
        for r in &ranges {
            let mut t = toml_edit::Table::new();
            t["interface"] = toml_edit::value(r.interface.clone());
            t["start"] = toml_edit::value(r.start.clone());
            t["end"] = toml_edit::value(r.end.clone());
            t["gateway"] = toml_edit::value(r.gateway.clone());
            t["cidr"] = toml_edit::value(i64::from(r.cidr));
            let mut dns = toml_edit::Array::new();
            for d in &r.dns_servers {
                dns.push(d.as_str());
            }
            t["dns_servers"] = toml_edit::value(dns);
            aot.push(t);
        }
        doc["dhcp"]["ranges"] = toml_edit::Item::ArrayOfTables(aot);
    }

    let out = doc.to_string();
    // Never write something the daemon cannot load.
    toml::from_str::<AppConfig>(&out).map_err(|e| format!("generated config is invalid: {e}"))?;
    std::fs::write(config_path, out).map_err(|e| format!("failed to write {config_path}: {e} (run as root)"))?;

    println!();
    println!("Saved {config_path}");
    if generated_token {
        println!("Generated API token (also the web UI login): {token}");
    }
    let port = api_bind.rsplit(':').next().unwrap_or("8686");
    println!("Web UI: http://<node-ip>:{port}/ui/");
    if !dhcp_enabled {
        println!("DHCP is disabled; run `feathertail configure` again to enable it.");
    }

    if Path::new("/run/systemd/system").exists() && ask_yes("Restart the feathertail service now?", true)? {
        let _ = Command::new("systemctl").args(["enable", "feathertail.service"]).status();
        match Command::new("systemctl").args(["restart", "feathertail.service"]).status() {
            Ok(s) if s.success() => println!("Service restarted. Logs: journalctl -u feathertail -f"),
            _ => println!("Restart failed; check: systemctl status feathertail"),
        }
    } else {
        println!("Apply with: sudo systemctl restart feathertail");
    }
    Ok(())
}

fn ask_range() -> Result<DhcpRange, String> {
    let bridges = list_bridges();
    if !bridges.is_empty() {
        let shown: Vec<String> = bridges
            .iter()
            .map(|b| match bridge_ipv4(b) {
                Some((ip, cidr)) => format!("{b} ({ip}/{cidr})"),
                None => b.clone(),
            })
            .collect();
        println!("Bridges on this host: {}", shown.join(", "));
    }
    loop {
        let default_if = bridges.first().cloned().unwrap_or_default();
        let interface = ask("Interface to serve DHCP on (empty = any)", &default_if)?;
        let detected = (!interface.is_empty()).then(|| bridge_ipv4(&interface)).flatten();
        let gateway = ask(
            "Gateway (router) for clients",
            &detected.map(|(ip, _)| ip.to_string()).unwrap_or_default(),
        )?;
        let cidr_default = detected.map(|(_, c)| c).unwrap_or(24).to_string();
        let cidr: u8 = match ask("Prefix length (CIDR, 24 = 255.255.255.0)", &cidr_default)?.parse() {
            Ok(c) if c <= 30 => c,
            _ => {
                println!("  invalid CIDR, try again");
                continue;
            }
        };
        let (start_default, end_default) = gateway
            .parse::<Ipv4Addr>()
            .map(|gw| default_span(gw, cidr))
            .unwrap_or_default();
        let start = ask("First IP to hand out", &start_default)?;
        let end = ask("Last IP to hand out", &end_default)?;
        let dns = ask("DNS servers (comma separated)", "1.1.1.1, 8.8.8.8")?;
        let range = DhcpRange {
            interface,
            start,
            end,
            gateway,
            cidr,
            dns_servers: dns
                .split([',', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            server_ip: None,
            lease_time_secs: None,
        };
        match crate::dhcp::validate_range(&range) {
            Ok(()) => return Ok(range),
            Err(err) => println!("  {err}; try again"),
        }
    }
}

/// .100-.200 for /24 and larger networks, otherwise every host address.
fn default_span(gateway: Ipv4Addr, cidr: u8) -> (String, String) {
    let mask = if cidr == 0 { 0 } else { u32::MAX << (32 - u32::from(cidr)) };
    let net = u32::from(gateway) & mask;
    let bcast = net | !mask;
    let (start, end) = if bcast - net >= 255 {
        (net + 100, net + 200)
    } else {
        (net + 1, bcast - 1)
    };
    (Ipv4Addr::from(start).to_string(), Ipv4Addr::from(end).to_string())
}

fn iface_label(iface: &str) -> String {
    if iface.is_empty() { "any interface:".into() } else { format!("{iface}:") }
}

fn list_bridges() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("vmbr"))
        .collect();
    out.sort();
    out
}

/// First IPv4 address of an interface, via `ip -4 -o addr show dev <iface>`.
fn bridge_ipv4(iface: &str) -> Option<(Ipv4Addr, u8)> {
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", iface])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut words = text.split_whitespace();
    words.find(|w| *w == "inet")?;
    let (ip, cidr) = words.next()?.split_once('/')?;
    Some((ip.parse().ok()?, cidr.parse().ok()?))
}

fn random_token() -> Result<String, String> {
    let mut raw = [0u8; 24];
    getrandom::getrandom(&mut raw).map_err(|e| e.to_string())?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

fn ask(label: &str, default: &str) -> Result<String, String> {
    if default.is_empty() {
        print!("{label}: ");
    } else {
        print!("{label} [{default}]: ");
    }
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => Err("input closed, nothing was saved".to_owned()),
        Ok(_) => {
            let v = line.trim();
            Ok(if v.is_empty() { default.to_owned() } else { v.to_owned() })
        }
    }
}

fn ask_yes(label: &str, default: bool) -> Result<bool, String> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        match ask(&format!("{label} ({hint})"), "")?.to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" | "d" | "da" => return Ok(true),
            "n" | "no" | "nu" => return Ok(false),
            _ => println!("  answer y or n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_span_picks_sane_ranges() {
        assert_eq!(
            default_span(Ipv4Addr::new(10, 0, 0, 1), 24),
            ("10.0.0.100".into(), "10.0.0.200".into())
        );
        assert_eq!(
            default_span(Ipv4Addr::new(192, 168, 1, 1), 28),
            ("192.168.1.1".into(), "192.168.1.14".into())
        );
    }
}
