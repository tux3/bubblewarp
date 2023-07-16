use crate::namespace;
use crate::namespace::{is_mounted, run_inside_namespace, Type};
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;
use strum::IntoEnumIterator;

pub fn down() -> Result<()> {
    let base_dir = namespace::base_dir()?;

    if is_mounted(&base_dir, Type::Mount)? {
        clean_mount_namespace(&base_dir)?;
    }

    cleanup_private_networking(&base_dir)?;

    // TODO:
    //  - Cleanup external networking (eth/wifi)
    //  - If the namespaces exist, kill any processes running inside any of the namespaces!
    unmount_namespaces(&base_dir)?;
    let _ = nix::mount::umount(&base_dir);
    unimplemented!()
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
    for ns_type in namespace::Type::iter() {
        if is_mounted(base_dir, ns_type)? {
            unmount_one_namespace(base_dir, ns_type)?;
        }
    }
    Ok(())
}

pub fn unmount_one_namespace(base_dir: &Path, ns_type: namespace::Type) -> Result<()> {
    let ns_mount_point = namespace::mount_point(base_dir, ns_type);
    nix::mount::umount(&ns_mount_point).context("Unmounting persistent namespace")?;
    Ok(())
}
