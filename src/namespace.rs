use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
