use crate::namespace;
use crate::namespace::is_mounted;
use anyhow::{Context, Result};
use std::path::Path;
use strum::IntoEnumIterator;

pub fn down() -> Result<()> {
    let base_dir = namespace::base_dir()?;

    // TODO:
    //  - If the namespaces exist, kill any processes running inside any of the namespaces!
    //  - If the mount namespace exists, unmount the /etc overlay inside it (if mounted)
    unmount_namespaces(&base_dir)?;
    nix::mount::umount(&base_dir).context("Unmounting the base directory self bind mount")?;
    unimplemented!()
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
