use crate::namespace::{self, find_process_in_namespace, is_mounted, run_inside_namespace, Type};
use crate::net::default_route_iface_name;
use anyhow::{bail, Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::path::Path;
use std::process::{Command, Stdio};
use strum::IntoEnumIterator;
use tracing::debug;

pub fn down() -> Result<()> {
    let base_dir = namespace::base_dir()?;

    kill_process("warp-svc")?;
    kill_process("danted")?;
    kill_process("danted: io-chil")?;

    if is_mounted(&base_dir, Type::Mount)? {
        clean_mount_namespace(&base_dir)?;
    }
    if is_mounted(&base_dir, Type::Net)? {
        cleanup_external_networking()?;
    }
    cleanup_private_networking(&base_dir)?;

    unmount_namespaces(&base_dir)?;
    let _ = nix::mount::umount(&base_dir);
    Ok(())
}

fn kill_process(name: &str) -> Result<()> {
    if let Some(pid) = find_process_in_namespace(name)? {
        debug!("Killing running {name} process");
        kill(Pid::from_raw(pid as i32), Signal::SIGTERM)?;
    }
    Ok(())
}

fn cleanup_external_networking() -> Result<()> {
    let iface_name = default_route_iface_name()?;
    delete_iptables_rule(&format!(
        "POSTROUTING -t nat -s 10.200.0.0/24 -o {iface_name} -j MASQUERADE"
    ));
    delete_iptables_rule(&format!("FORWARD -i {iface_name} -o veth-warp -j ACCEPT"));
    delete_iptables_rule(&format!("FORWARD -o {iface_name} -i veth-warp -j ACCEPT"));
    Ok(())
}

fn delete_iptables_rule(rule: &str) {
    let rule_words: Vec<&str> = rule.split(' ').collect();
    loop {
        let status = Command::new("/usr/sbin/iptables")
            .arg("-D")
            .args(&rule_words)
            .stderr(Stdio::null())
            .status()
            .unwrap();
        if !status.success() {
            break;
        }
    }
}

fn cleanup_private_networking(base_dir: &Path) -> Result<()> {
    if is_mounted(base_dir, Type::Net)? {
        let _ = run_inside_namespace(
            base_dir,
            Type::Mount,
            Command::new("ip").args(["link", "delete", "dev", "veth-warp-ns"]),
        );
    }

    if nix::ifaddrs::getifaddrs()?.any(|dev| dev.interface_name == "veth-warp") {
        let out = Command::new("ip")
            .args(["link", "delete", "dev", "veth-warp"])
            .output()?;
        if !out.status.success() {
            bail!(
            "Failed to delete private veth network interface, returned {}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
        }
    }
    Ok(())
}

fn clean_mount_namespace(base_dir: &Path) -> Result<()> {
    let _ = run_inside_namespace(base_dir, Type::Mount, Command::new("umount").arg("/proc"));
    let _ = run_inside_namespace(base_dir, Type::Mount, Command::new("umount").arg("/etc"));
    Ok(())
}

pub fn unmount_namespaces(base_dir: &Path) -> Result<()> {
    for ns_type in Type::iter() {
        if is_mounted(base_dir, ns_type)? {
            unmount_one_namespace(base_dir, ns_type)?;
        }
    }
    Ok(())
}

pub fn unmount_one_namespace(base_dir: &Path, ns_type: Type) -> Result<()> {
    let ns_mount_point = namespace::mount_point(base_dir, ns_type);
    nix::mount::umount(&ns_mount_point).context("Unmounting persistent namespace")?;
    Ok(())
}
