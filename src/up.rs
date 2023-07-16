use crate::namespace;
use crate::namespace::{
    mount_point, run_inside_namespace, spawn_inside_all_namespaces, Status, Type,
};
use crate::net::{setup_external_networking, setup_private_networking};
use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
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
        Status::Partial(_mounted_set) => {
            bail!("Namespaces partially mounted! Try calling the down command again");
        }
        Status::None => {
            create_namespaces(&base_dir)?;
        }
    }

    create_etc_overlay_inside(&base_dir)?;
    setup_private_networking(&base_dir)?;
    setup_external_networking(&base_dir)?;
    spawn_process_inside(&base_dir, "warp-svc")?;

    // TODO: Wait for warp interface to be up inside the container instead of a hard sleep..
    std::thread::sleep(Duration::from_millis(1000));

    spawn_process_inside(&base_dir, "/usr/sbin/danted")?;

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
    use namespace::Type::*;

    debug!("Creating mount points for persistent namespaces");
    for ns_type in namespace::Type::iter() {
        let ns_mount_point = mount_point(base_dir, ns_type);
        if !ns_mount_point.exists() {
            File::create(ns_mount_point).context("Creating persistent namespace mount point")?;
        }
    }

    debug!("Calling unshare to create persistent namespaces");
    let output = Command::new("unshare")
        .arg("--fork")
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
    let overlay_dir = base_dir.join("etc_overlay");
    let extra_lower = overlay_dir.join("extra_lower");
    let upper = overlay_dir.join("upper");
    let work = overlay_dir.join("work");

    let mount_out = run_inside_namespace(base_dir, Type::Mount, &Command::new("mount"))?;
    if String::from_utf8_lossy(&mount_out.stdout).contains("overlay on /etc type overlay") {
        debug!("/etc overlay appears already mounted, not mounting it again");
        return Ok(());
    }

    std::fs::create_dir_all(&extra_lower)?;
    std::fs::create_dir_all(&upper)?;
    std::fs::create_dir_all(&work)?;

    {
        let resolv_path = extra_lower.join("resolv.conf");
        let resolv_file_data = b"# This is a hardcoded overlay of resolv.conf in the WARP container
nameserver 127.0.2.2
nameserver 127.0.2.3
nameserver fd01:db8:1111::2
nameserver fd01:db8:1111::3
";
        let mut f = File::create(resolv_path)?;
        f.write_all(resolv_file_data)?;
    }

    {
        let danted_path = extra_lower.join("danted.conf");
        let file_data = b"internal: 10.200.0.2 port = 8080
external: CloudflareWARP
socksmethod: none
clientmethod: none
client pass { from: 0.0.0.0/0 to: 0.0.0.0/0 }
socks pass { from: 0.0.0.0/0 to: 0.0.0.0/0 }
";
        let mut f = File::create(danted_path)?;
        f.write_all(file_data)?;
    }

    debug!("Mount read-only /etc overlay inside namespace");
    let opt_lower = format!("lowerdir={}:/etc", extra_lower.to_string_lossy());
    let opt_upper = format!("upperdir={}", upper.to_string_lossy());
    let opt_work = format!("workdir={}", work.to_string_lossy());
    let mut cmd = Command::new("mount");
    cmd.args(["-t", "overlay", "overlay"])
        .arg(format!("-o{opt_lower},{opt_upper},{opt_work}"))
        .arg("/etc");
    run_inside_namespace(base_dir, namespace::Type::Mount, &cmd)?;

    Ok(())
}

pub fn spawn_process_inside(base_dir: &Path, name: &str) -> Result<()> {
    for proc in procfs::process::all_processes()? {
        let Ok(proc) = proc else { continue };
        let Ok(cmdline) = proc.cmdline() else {
            continue;
        };
        if cmdline.is_empty() || !cmdline[0].ends_with(name) {
            continue;
        }
        warn!("There appears to already be a {name} process running, not starting another");
        return Ok(());
    }

    debug!("Spawning {name} process inside namespaces");
    spawn_inside_all_namespaces(base_dir, &Command::new(name))?;
    Ok(())
}
