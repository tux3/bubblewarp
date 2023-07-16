use crate::namespace::{mount_point, run_inside_namespace, Type};
use anyhow::{bail, Result};
use std::path::Path;
use std::process::Command;
use tracing::debug;

fn parse_iface_name(ip_route_default_stdout: Vec<u8>) -> Result<String> {
    let out = String::from_utf8(ip_route_default_stdout)?;
    let parts: Vec<&str> = out.split(' ').collect();
    if parts.is_empty() {
        bail!("Empty stdout when running ip route show default!")
    }
    let pos_dev = parts.iter().position(|&e| e == "dev");
    if parts[0] != "default" || pos_dev.is_none() || parts.len() <= pos_dev.unwrap() + 1 {
        bail!("Unexpected output from ip route show default")
    }
    Ok(parts[pos_dev.unwrap() + 1].to_owned())
}

pub fn container_has_default_route(base_dir: &Path) -> Result<bool> {
    let out = run_inside_namespace(
        base_dir,
        Type::Net,
        Command::new("ip").args(["route", "show", "default"]),
    )?;
    Ok(!out.stdout.is_empty())
}

pub fn default_route_iface_name() -> Result<String> {
    let out = Command::new("ip")
        .args(["route", "show", "default"])
        .output()?;
    out.status.exit_ok()?;
    parse_iface_name(out.stdout)
}

pub fn setup_private_networking(base_dir: &Path) -> Result<()> {
    if nix::ifaddrs::getifaddrs()?.any(|dev| dev.interface_name == "veth-warp") {
        debug!("veth-warp iface seems to already exist, not re-creating it");
        return Ok(());
    }

    debug!("Setting up veth pair for private networking");
    let net_ns = mount_point(base_dir, Type::Net);
    Command::new("ip")
        .args(["link", "add", "veth-warp", "type", "veth"])
        .args(["peer", "name", "veth-warp-ns"])
        .args(["netns", net_ns.to_string_lossy().as_ref()])
        .status()?
        .exit_ok()?;
    Command::new("ip")
        .args(["addr", "add", "10.200.0.1/24", "dev", "veth-warp"])
        .status()?
        .exit_ok()?;
    Command::new("ip")
        .args(["link", "set", "veth-warp", "up"])
        .status()?
        .exit_ok()?;

    run_inside_namespace(
        base_dir,
        Type::Net,
        Command::new("ip").args(["addr", "add", "10.200.0.2/24", "dev", "veth-warp-ns"]),
    )?;
    run_inside_namespace(
        base_dir,
        Type::Net,
        Command::new("ip").args(["link", "set", "veth-warp-ns", "up"]),
    )?;
    Ok(())
}

pub fn setup_external_networking(base_dir: &Path) -> Result<()> {
    if container_has_default_route(base_dir)? {
        debug!(
            "Container appears to already have default route, keeping external networking as-is"
        );
        return Ok(());
    }

    let iface_name = default_route_iface_name()?;
    setup_external_forward(base_dir, &iface_name)?;
    Ok(())
}

pub fn setup_external_forward(base_dir: &Path, iface_name: &str) -> Result<()> {
    debug!("Setting up external forward for interface {iface_name}");
    Command::new("/usr/sbin/iptables")
        .args(["-t", "nat", "-A", "POSTROUTING", "-s", "10.200.0.2/24"])
        .args(["-o", iface_name, "-j", "MASQUERADE"])
        .status()?
        .exit_ok()?;
    Command::new("/usr/sbin/iptables")
        .args(["-A", "FORWARD", "-i", iface_name, "-o", "veth-warp"])
        .args(["-j", "ACCEPT"])
        .status()?
        .exit_ok()?;
    Command::new("/usr/sbin/iptables")
        .args(["-A", "FORWARD", "-o", iface_name, "-i", "veth-warp"])
        .args(["-j", "ACCEPT"])
        .status()?
        .exit_ok()?;

    run_inside_namespace(
        base_dir,
        Type::Net,
        Command::new("ip")
            .args(["route", "add", "default"])
            .args(["via", "10.200.0.1"])
            .args(["dev", "veth-warp-ns"]),
    )?;
    Ok(())
}
