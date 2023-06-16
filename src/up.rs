use crate::namespace;
use crate::namespace::Status;
use anyhow::{bail, Context, Result};
use std::path::Path;
use strum::IntoEnumIterator;
use tracing::{debug, info, trace, warn};

pub fn up() -> Result<()> {
    // TODO: Check that /etc/subuid and /etc/subgid contain a suitable range, and if not warn about it...
    let base_dir = namespace::base_dir()?;
    if !base_dir.exists() {
        std::fs::create_dir_all(&base_dir)?;
    }

    if base_dir_has_private_self_bind_mount(&base_dir)? {
        warn!("Persistent namespace base directory is still bind-mounted, continuing...")
    } else {
        private_self_bind_mount_base_dir(&base_dir)?;
    }

    match namespace::status(&base_dir)? {
        Status::Ready => {
            info!("Namespaces already mounted")
        }
        Status::Partial(mounted_set) => {
            warn!("Namespaces partially mounted, cleaning up and re-creating the namespaces");
            todo!(
                "call all the down functions _EXCEPT_ destroying the private self bind mount here"
            );
            create_namespaces(&base_dir)?;
        }
        Status::None => {
            create_namespaces(&base_dir)?;
        }
    }

    create_etc_overlay_inside(&base_dir)?;
    Ok(())
}

pub fn base_dir_has_private_self_bind_mount(base_dir: &Path) -> Result<bool> {
    use procfs::process::Process;

    let base_dir = base_dir.canonicalize()?;
    for mount in Process::myself()?.mountinfo()? {
        let root = if let Ok(root) = std::fs::canonicalize(&mount.root) {
            root
        } else {
            continue;
        };
        if root != base_dir {
            continue;
        }

        let dst = if let Ok(dst) = std::fs::canonicalize(&mount.mount_point) {
            dst
        } else {
            continue;
        };
        if dst != base_dir {
            continue;
        }
        trace!("Found base dir self bind mount point: {:#?}", mount);
        return Ok(true);
    }
    Ok(false)
}

pub fn private_self_bind_mount_base_dir(base_dir: &Path) -> Result<()> {
    use nix::mount::MsFlags;

    debug!("Creating base dir private self bind mount");
    nix::mount::mount(
        Some(base_dir),
        base_dir,
        None::<&Path>,
        MsFlags::MS_BIND,
        None::<&Path>,
    )?;

    if let Err(e) = nix::mount::mount(
        None::<&Path>,
        base_dir,
        None::<&Path>,
        MsFlags::MS_PRIVATE,
        None::<&Path>,
    ) {
        let _ = nix::mount::umount(base_dir);
        return Err(e.into());
    }

    Ok(())
}

pub fn create_namespaces(base_dir: &Path) -> Result<()> {
    use namespace::mount_point;
    use namespace::Type::*;
    use std::process::Command;

    debug!("Creating mount points for persistent namespaces");
    for ns_type in namespace::Type::iter() {
        let ns_mount_point = mount_point(base_dir, ns_type);
        if !ns_mount_point.exists() {
            std::fs::File::create(ns_mount_point)
                .context("Creating persistent namespace mount point")?;
        }
    }

    debug!("Calling unshare to create persistent namespaces");
    let output = Command::new("unshare")
        .arg("--fork")
        .arg("--mount-proc")
        .arg("-r")
        .arg("--map-users=0,0,1200")
        .arg("--map-groups=0,0,1200")
        .arg(format!("--pid={}", mount_point(base_dir, Pid).display()))
        .arg(format!("--user={}", mount_point(base_dir, User).display()))
        .arg(format!("--net={}", mount_point(base_dir, Net).display()))
        .arg(format!(
            "--mount={}",
            mount_point(base_dir, Mount).display()
        ))
        .args(["--", "echo", "ok"])
        .output()?;
    if !output.status.success() {
        bail!(
            "Failed to create namespaces, unshare returned {}\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    }
    Ok(())
}

pub fn create_etc_overlay_inside(base_dir: &Path) -> Result<()> {
    todo!("Enter and create overlay /etc rw mount inside the namespace")
}
