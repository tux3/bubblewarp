use anyhow::{anyhow, bail, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use strum::{EnumCount, IntoEnumIterator};
use strum_macros::{EnumCount as EnumCountMacro, EnumIter};
use tracing::trace;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, EnumIter, EnumCountMacro)]
pub enum Type {
    User,
    Pid,
    Mount,
    Net,
}

impl ToString for Type {
    fn to_string(&self) -> String {
        match self {
            Type::User => "user",
            Type::Pid => "pid",
            Type::Mount => "mount",
            Type::Net => "net",
        }
        .to_owned()
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Status {
    Ready,
    Partial(HashSet<Type>),
    None,
}

pub fn base_dir() -> Result<PathBuf> {
    let project_dirs = directories::ProjectDirs::from("", "", "bubblewarp")
        .ok_or_else(|| anyhow!("Failed to get the path of our data directory"))?;
    Ok(project_dirs.data_dir().to_owned())
}

pub fn status(base_dir: &Path) -> Result<Status> {
    let mut mounted_set = HashSet::new();

    for ns_type in Type::iter() {
        if is_mounted(base_dir, ns_type)? {
            mounted_set.insert(ns_type);
        }
    }

    if mounted_set.is_empty() {
        Ok(Status::None)
    } else if mounted_set.len() == Type::COUNT {
        Ok(Status::Ready)
    } else {
        Ok(Status::Partial(mounted_set))
    }
}

pub fn is_mounted(base_dir: &Path, ns_type: Type) -> Result<bool> {
    use procfs::process::Process;

    let mut ns_mount_point = mount_point(base_dir, ns_type);
    if !ns_mount_point.exists() {
        return Ok(false);
    }
    if let Ok(canon) = ns_mount_point.canonicalize() {
        ns_mount_point = canon;
    }

    for mount in Process::myself()?.mountinfo()? {
        if mount.fs_type != "nsfs" || mount.mount_source.as_deref() != Some("nsfs") {
            continue;
        }

        let dst = if let Ok(dst) = std::fs::canonicalize(&mount.mount_point) {
            dst
        } else {
            continue;
        };
        if dst != ns_mount_point {
            continue;
        }

        trace!(
            "Found mounted persistent namespace at {}",
            ns_mount_point.display()
        );
        return Ok(true);
    }
    Ok(false)
}

pub fn mount_point(base_dir: &Path, ns_type: Type) -> PathBuf {
    let mut path = base_dir.to_owned();
    match ns_type {
        Type::User => path.push("user"),
        Type::Pid => path.push("pid"),
        Type::Mount => path.push("mount"),
        Type::Net => path.push("net"),
    };
    path
}

fn wrap_cmd(wrapper: &mut Command, cmd: &Command) {
    wrapper.arg(cmd.get_program());
    wrapper.args(cmd.get_args());
    if let Some(cwd) = cmd.get_current_dir() {
        wrapper.current_dir(cwd);
    }
}

pub fn run_inside_namespace(base_dir: &Path, ns_type: Type, cmd: &Command) -> Result<Output> {
    let mut ns_cmd = Command::new("nsenter");
    ns_cmd.arg(format!(
        "--{}={}",
        ns_type.to_string(),
        mount_point(base_dir, ns_type).to_string_lossy()
    ));
    wrap_cmd(&mut ns_cmd, cmd);

    let out = ns_cmd.output()?;
    if !out.status.success() {
        bail!(
            "Failed to run command {} inside namespaces, returned {}\nstdout: {}\nstderr: {}",
            cmd.get_program().to_string_lossy(),
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
    }
    Ok(out)
}

pub fn spawn_inside_all_namespaces(cmd: &Command, ns_pid: u32) -> Result<Child> {
    let mut ns_cmd = Command::new("nsenter");
    ns_cmd.args(["-a", "-t", &ns_pid.to_string()]);
    ns_cmd.stdout(Stdio::null());
    ns_cmd.stderr(Stdio::null());
    wrap_cmd(&mut ns_cmd, cmd);
    let child = ns_cmd.spawn()?;
    Ok(child)
}

pub fn all_ns_processes(base_dir: &Path) -> Result<impl Iterator<Item = procfs::process::Process>> {
    let pid_ns_id = std::fs::metadata(mount_point(base_dir, Type::Pid))?.ino();
    Ok(procfs::process::all_processes()?.filter_map(move |p| {
        let Ok(p) = p else { return None };
        if p.namespaces().is_ok_and(|map| {
            map.get(OsStr::new("pid"))
                .is_some_and(|pid_ns| pid_ns.identifier == pid_ns_id)
        }) {
            Some(p)
        } else {
            None
        }
    }))
}
