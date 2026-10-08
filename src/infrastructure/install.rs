use crate::application::ports::{
    CleanupWarning, InstalledSkillSource, PreparedSkill, PreparedSkillInstaller, SkillInstallError,
    SkillInstallRequest, SkillInstaller, SkillRollbackFailure,
};
use crate::domain::lockfile::LockedFile;
use crate::domain::package_filter::PackageFilter;
use crate::domain::skill::SkillName;
use crate::domain::skill_manifest::parse_skill_manifest;
use crate::domain::source::{GitInstallSource, InstallSource};
use crate::infrastructure::git::{GitClient, GitCommandError};
use crate::infrastructure::hash::hash_directory;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct FileSystemSkillInstaller;

impl SkillInstaller for FileSystemSkillInstaller {
    fn install_skill(
        &self,
        request: &SkillInstallRequest,
        destination: &Path,
        skill_name: &str,
    ) -> Result<InstalledSkillSource, SkillInstallError> {
        install_prepared(self.prepare_skill(request, destination, skill_name)?)
    }
}

fn install_prepared(
    mut prepared: PreparedSkill<SkillInstallReceipt>,
) -> Result<InstalledSkillSource, SkillInstallError> {
    let installer = FileSystemSkillInstaller;
    if let Err(original) = installer.publish_skill(&mut prepared.receipt) {
        return match installer.rollback_skill(&mut prepared.receipt) {
            Ok(()) => Err(original),
            Err(failure) => Err(SkillInstallError::RollbackFailed {
                original: Box::new(original),
                failure,
            }),
        };
    }
    prepared
        .installed
        .warnings
        .extend(installer.finalize_skill(&mut prepared.receipt));
    Ok(prepared.installed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationState {
    Prepared,
    OldMoved,
    Published,
    RolledBack,
    Finalized,
}

/// Opaque ownership of a single directory cutover, not a crash-recovery journal.
#[derive(Debug)]
pub struct SkillInstallReceipt {
    name: SkillName,
    managed_root: PathBuf,
    destination: PathBuf,
    original: Option<DirectoryIdentity>,
    staging: OwnedDirectory,
    backup: Option<OwnedDirectory>,
    state: PublicationState,
    #[cfg(test)]
    faults: InstallFaults,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct InstallFaults {
    publish_after_backup: bool,
    rollback: bool,
    finalize: bool,
}

// Holding a handle prevents inode reuse from making a replacement look owned.
#[derive(Debug)]
struct DirectoryIdentity(fs::File);

impl DirectoryIdentity {
    fn read(path: &Path) -> std::io::Result<Option<Self>> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => fs::File::open(path).map(Self).map(Some),
            Ok(_) => Err(std::io::Error::other(
                "expected a directory, not a symlink or other file",
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn matches(&self, path: &Path) -> std::io::Result<bool> {
        use std::os::unix::fs::MetadataExt;
        match fs::symlink_metadata(path) {
            Ok(current) if current.is_dir() => {
                let owned = self.0.metadata()?;
                Ok(current.dev() == owned.dev() && current.ino() == owned.ino())
            }
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[derive(Debug)]
struct OwnedDirectory {
    path: PathBuf,
    identity: DirectoryIdentity,
}

impl OwnedDirectory {
    fn create(parent: &Path, kind: &str, destination: &Path) -> std::io::Result<Self> {
        Self::create_excluding(parent, kind, &[destination])
    }

    fn create_excluding(parent: &Path, kind: &str, excluded: &[&Path]) -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
        loop {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let suffix = format!("-{sequence}");
            // Skip the final name's ASCII sequence suffix regardless of prefix.
            // Unicode filesystem aliases can change prefix spelling, so matching
            // the complete candidate name (even ASCII-insensitively) is unsafe.
            if excluded.iter().any(|destination| {
                destination.file_name().is_some_and(|final_name| {
                    final_name.as_encoded_bytes().ends_with(suffix.as_bytes())
                })
            }) {
                continue;
            }
            let name = format!(".sksync-{kind}-{}{suffix}", std::process::id());
            let path = parent.join(name);
            match fs::create_dir(&path) {
                Ok(()) => {
                    let identity = DirectoryIdentity::read(&path)
                        .map_err(|error| {
                            std::io::Error::other(format!(
                                "private directory identity failed; retained path {}: {error}",
                                path.display()
                            ))
                        })?
                        .ok_or_else(|| {
                            std::io::Error::other(format!(
                                "new private directory disappeared: {}",
                                path.display()
                            ))
                        })?;
                    return Ok(Self { path, identity });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    fn remove(&self) -> std::io::Result<()> {
        if !self.identity.matches(&self.path)? {
            return Err(std::io::Error::other(
                "owned directory changed; refusing cleanup",
            ));
        }
        fs::remove_dir_all(&self.path)
    }
}

/// Called under the managed-store writer guard, before any skill preparation.
/// Probe only missing components beside their actual nearest existing container,
/// never by creating an absent selected body or entering an existing one.
pub(crate) fn validate_update_destination_aliases(
    managed_root: &Path,
    destinations: &[(&SkillName, &Path)],
) -> Result<(), SkillInstallError> {
    validate_update_destination_aliases_inner(
        managed_root,
        destinations,
        #[cfg(test)]
        false,
    )
}

fn validate_update_destination_aliases_inner(
    managed_root: &Path,
    destinations: &[(&SkillName, &Path)],
    #[cfg(test)] fail_cleanup: bool,
) -> Result<(), SkillInstallError> {
    if destinations.len() < 2 {
        return Ok(());
    }
    let root = managed_root
        .canonicalize()
        .map_err(|error| prepare_io(managed_root, error))?;
    let mut groups: Vec<UpdateDestinationProbe> = Vec::new();
    for (index, (_, path)) in destinations.iter().enumerate() {
        validate_managed_destination(&root, path)?;
        let physical = resolve_managed_directory(path)?;
        let mut parent = physical.as_path();
        let identity = loop {
            if let Some(identity) =
                DirectoryIdentity::read(parent).map_err(|error| prepare_io(parent, error))?
            {
                break identity;
            }
            parent = parent
                .parent()
                .ok_or_else(|| prepare_io(path, "missing alias probe container"))?;
        };
        let relative = physical
            .strip_prefix(parent)
            .map_err(|error| prepare_io(path, error))?
            .to_path_buf();
        if !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(prepare_io(
                path,
                "unsupported destination components for alias validation",
            ));
        }
        let mut matching_group = None;
        for (group_index, group) in groups.iter().enumerate() {
            if group
                .identity
                .matches(parent)
                .map_err(|error| prepare_io(parent, error))?
            {
                matching_group = Some(group_index);
                break;
            }
        }
        if let Some(group_index) = matching_group {
            groups[group_index].destinations.push((index, relative));
        } else {
            groups.push(UpdateDestinationProbe {
                parent: parent.to_path_buf(),
                identity,
                destinations: vec![(index, relative)],
            });
        }
    }
    for group in groups {
        if group.destinations.len() < 2 {
            continue;
        }
        if let Some(existing) = group
            .destinations
            .iter()
            .position(|(_, path)| path.as_os_str().is_empty())
        {
            let other = if existing == 0 { 1 } else { 0 };
            return Err(update_destination_overlap(
                &root,
                destinations,
                group.destinations[existing].0,
                group.destinations[other].0,
            ));
        }
        if !group
            .identity
            .matches(&group.parent)
            .map_err(|error| prepare_io(&group.parent, error))?
        {
            return Err(prepare_io(&group.parent, "alias probe container changed"));
        }
        // Every first missing component is excluded before allocation. Numeric
        // suffix matching remains safe for case/Kelvin-sign/long-s aliases.
        let excluded = group
            .destinations
            .iter()
            .map(|(_, path)| {
                group.parent.join(
                    path.components()
                        .next()
                        .expect("nonempty missing suffix")
                        .as_os_str(),
                )
            })
            .collect::<Vec<_>>();
        let excluded = excluded.iter().map(PathBuf::as_path).collect::<Vec<_>>();
        let probe =
            OwnedDirectory::create_excluding(&group.parent, "destination-probe-é", &excluded)
                .map_err(|error| prepare_io(&group.parent, error))?;
        let result = probe_update_destination_group(&probe, &group, destinations);
        let cleanup = || {
            #[cfg(test)]
            if fail_cleanup {
                return Err(std::io::Error::other(
                    "injected alias probe cleanup failure",
                ));
            }
            probe.remove()
        };
        if let Err(cleanup) = cleanup() {
            let original = result
                .err()
                .map(|error| format!("{error}; "))
                .unwrap_or_default();
            return Err(prepare_io(&probe.path, format!("{original}alias probe cleanup failed: {cleanup}; retained owned recovery path {}", probe.path.display())));
        }
        result?;
    }
    Ok(())
}

struct UpdateDestinationProbe {
    parent: PathBuf,
    identity: DirectoryIdentity,
    destinations: Vec<(usize, PathBuf)>,
}

fn update_destination_overlap(
    root: &Path,
    destinations: &[(&SkillName, &Path)],
    left: usize,
    right: usize,
) -> SkillInstallError {
    prepare_io(root, format!(
        "dependency destinations overlap: '{}' ({}) and '{}' ({}); use separate managed body directories",
        destinations[left].0, destinations[left].1.display(), destinations[right].0, destinations[right].1.display()
    ))
}

fn probe_update_destination_group(
    probe: &OwnedDirectory,
    group: &UpdateDestinationProbe,
    destinations: &[(&SkillName, &Path)],
) -> Result<(), SkillInstallError> {
    // A per-directory case/normalization policy need not be inherited by a new
    // private directory. Verify inheritance against lookups in the real parent;
    // otherwise the mirror cannot establish physical equivalence safely.
    let basename = probe
        .path
        .file_name()
        .expect("owned probe name")
        .to_str()
        .expect("generated UTF-8 name");
    let marker = probe.path.join(basename);
    fs::create_dir(&marker).map_err(|error| prepare_io(&marker, error))?;
    let marker_identity = DirectoryIdentity::read(&marker)
        .map_err(|error| prepare_io(&marker, error))?
        .ok_or_else(|| prepare_io(&marker, "alias marker disappeared"))?;
    for alias in [
        basename.to_uppercase(),
        basename.replace('k', "K"),
        basename.replace('s', "ſ"),
        basename.replace('é', "e\u{301}"),
    ] {
        let parent_alias = probe
            .identity
            .matches(&probe.path.with_file_name(&alias))
            .map_err(|error| prepare_io(&group.parent, error))?;
        let private_alias = marker_identity
            .matches(&marker.with_file_name(&alias))
            .map_err(|error| prepare_io(&marker, error))?;
        if parent_alias != private_alias {
            return Err(prepare_io(&group.parent, "cannot establish inherited directory alias behavior; update aborted before preparation"));
        }
    }
    fs::remove_dir(&marker).map_err(|error| prepare_io(&marker, error))?;
    let mut mirrored = Vec::new();
    for (_, path) in &group.destinations {
        let mut target = probe.path.clone();
        for component in path.components() {
            if !probe
                .identity
                .matches(&probe.path)
                .map_err(|error| prepare_io(&probe.path, error))?
                || !group
                    .identity
                    .matches(&group.parent)
                    .map_err(|error| prepare_io(&group.parent, error))?
                || !fs::symlink_metadata(&target)
                    .map_err(|error| prepare_io(&target, error))?
                    .is_dir()
            {
                return Err(prepare_io(
                    &target,
                    "owned alias probe changed; refusing traversal",
                ));
            }
            target.push(component.as_os_str());
            match fs::create_dir(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !fs::symlink_metadata(&target)
                        .map_err(|error| prepare_io(&target, error))?
                        .is_dir()
                    {
                        return Err(prepare_io(
                            &target,
                            "alias probe component is not a directory",
                        ));
                    }
                }
                Err(error) => return Err(prepare_io(&target, error)),
            }
        }
        let identity = DirectoryIdentity::read(&target)
            .map_err(|error| prepare_io(&target, error))?
            .ok_or_else(|| prepare_io(&target, "alias probe disappeared"))?;
        mirrored.push((target, identity));
    }
    for (index, (left, identity)) in mirrored.iter().enumerate() {
        for (other_index, (right, other_identity)) in mirrored.iter().enumerate().skip(index + 1) {
            let overlaps = right
                .ancestors()
                .map(|ancestor| identity.matches(ancestor))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|error| prepare_io(right, error))?
                .contains(&true)
                || left
                    .ancestors()
                    .map(|ancestor| other_identity.matches(ancestor))
                    .collect::<std::io::Result<Vec<_>>>()
                    .map_err(|error| prepare_io(left, error))?
                    .contains(&true);
            if overlaps {
                return Err(update_destination_overlap(
                    &group.parent,
                    destinations,
                    group.destinations[index].0,
                    group.destinations[other_index].0,
                ));
            }
        }
    }
    Ok(())
}

fn prepare_io(path: &Path, error: impl std::fmt::Display) -> SkillInstallError {
    SkillInstallError::Prepare {
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

impl PreparedSkillInstaller for FileSystemSkillInstaller {
    type Receipt = SkillInstallReceipt;

    fn prepare_skill(
        &self,
        request: &SkillInstallRequest,
        destination: &Path,
        skill_name: &str,
    ) -> Result<PreparedSkill<Self::Receipt>, SkillInstallError> {
        let name = SkillName::new(skill_name).map_err(|error| prepare_io(destination, error))?;
        fs::create_dir_all(&request.managed_root)
            .map_err(|error| prepare_io(&request.managed_root, error))?;
        validate_managed_destination(&request.managed_root, destination)?;
        let parent = destination
            .parent()
            .ok_or_else(|| prepare_io(destination, "missing destination parent"))?;
        fs::create_dir_all(parent).map_err(|error| prepare_io(parent, error))?;
        validate_managed_destination(&request.managed_root, destination)?;
        // Use the physical sibling parent for all receipt I/O; metadata keeps the logical final path.
        let physical_parent = parent
            .canonicalize()
            .map_err(|error| prepare_io(parent, error))?;
        let physical_destination = physical_parent.join(
            destination
                .file_name()
                .ok_or_else(|| prepare_io(destination, "missing body name"))?,
        );
        let root = request
            .managed_root
            .canonicalize()
            .map_err(|error| prepare_io(&request.managed_root, error))?;
        let original = DirectoryIdentity::read(&physical_destination)
            .map_err(|error| prepare_io(destination, error))?;
        let staging = OwnedDirectory::create(&physical_parent, "staging", &physical_destination)
            .map_err(|error| prepare_io(parent, error))?;
        let receipt = SkillInstallReceipt {
            name: name.clone(),
            managed_root: root,
            destination: physical_destination,
            original,
            staging,
            backup: None,
            state: PublicationState::Prepared,
            #[cfg(test)]
            faults: InstallFaults::default(),
        };
        let installed = install_to_staging(request, &receipt.staging.path)?;
        validate_skill_package(&receipt.staging.path)?;
        let hashes = hash_directory(&receipt.staging.path)
            .map_err(|error| prepare_io(&receipt.staging.path, error))?;
        Ok(PreparedSkill {
            name,
            destination: destination.to_path_buf(),
            installed,
            hash: hashes.hash,
            files: hashes
                .files
                .into_iter()
                .map(|file| LockedFile {
                    path: file.path,
                    hash: file.hash,
                })
                .collect(),
            receipt,
        })
    }

    fn publish_skill(&self, receipt: &mut Self::Receipt) -> Result<(), SkillInstallError> {
        let publish = |error| prepare_io(&receipt.destination, error);
        if receipt.state != PublicationState::Prepared {
            return Err(publish("receipt is not awaiting publication".into()));
        }
        validate_managed_destination(&receipt.managed_root, &receipt.destination)?;
        receipt
            .check_original()
            .map_err(|error| publish(error.to_string()))?;
        if !receipt
            .staging
            .identity
            .matches(&receipt.staging.path)
            .map_err(|error| publish(error.to_string()))?
        {
            return Err(publish("staging directory changed".into()));
        }
        if receipt.original.is_some() {
            let parent = receipt
                .staging
                .path
                .parent()
                .ok_or_else(|| publish("missing sibling parent".into()))?;
            let backup = OwnedDirectory::create(parent, "backup", &receipt.destination)
                .map_err(|error| publish(error.to_string()))?;
            receipt.backup = Some(backup);
            receipt
                .check_original()
                .map_err(|error| publish(error.to_string()))?;
            let old = receipt
                .backup_path()
                .ok_or_else(|| publish("missing backup path".into()))?;
            fs::rename(&receipt.destination, old).map_err(|error| publish(error.to_string()))?;
            receipt.state = PublicationState::OldMoved;
        }
        #[cfg(test)]
        if receipt.faults.publish_after_backup {
            return Err(publish("injected publication failure after backup".into()));
        }
        // Never replace an unexpected destination which appeared during cutover.
        require_absent(&receipt.destination).map_err(|error| publish(error.to_string()))?;
        fs::rename(&receipt.staging.path, &receipt.destination)
            .map_err(|error| publish(error.to_string()))?;
        receipt.state = PublicationState::Published;
        Ok(())
    }

    fn rollback_skill(&self, receipt: &mut Self::Receipt) -> Result<(), SkillRollbackFailure> {
        receipt.rollback().map_err(|error| SkillRollbackFailure {
            skill: receipt.name.clone(),
            retained_backup: receipt.backup_path().filter(|_| {
                matches!(
                    receipt.state,
                    PublicationState::OldMoved | PublicationState::Published
                )
            }),
            message: error.to_string(),
        })
    }

    fn finalize_skill(&self, receipt: &mut Self::Receipt) -> Vec<CleanupWarning> {
        if receipt.state != PublicationState::Published {
            return vec![CleanupWarning {
                path: receipt.destination.clone(),
                message: "receipt is not published; no cleanup performed".into(),
            }];
        }
        // Set the commit decision before cleanup: failure must never enable rollback.
        receipt.state = PublicationState::Finalized;
        if let Some(backup) = &receipt.backup {
            let cleanup = || -> std::io::Result<()> {
                #[cfg(test)]
                if receipt.faults.finalize {
                    return Err(std::io::Error::other("injected backup cleanup failure"));
                }
                receipt.check_backup()?;
                backup.remove()
            };
            if let Err(error) = cleanup() {
                return vec![CleanupWarning {
                    path: backup.path.clone(),
                    message: error.to_string(),
                }];
            }
        }
        Vec::new()
    }
}

fn require_absent(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(std::io::Error::other(
            "destination changed; refusing to overwrite it",
        )),
    }
}

impl SkillInstallReceipt {
    fn backup_path(&self) -> Option<PathBuf> {
        self.backup.as_ref().map(|backup| backup.path.join("body"))
    }

    fn check_original(&self) -> std::io::Result<()> {
        match &self.original {
            Some(identity) if identity.matches(&self.destination)? => Ok(()),
            Some(_) => Err(std::io::Error::other(
                "original destination changed; refusing publication",
            )),
            None => require_absent(&self.destination),
        }
    }

    fn check_backup(&self) -> std::io::Result<()> {
        let backup = self
            .backup
            .as_ref()
            .ok_or_else(|| std::io::Error::other("backup is missing"))?;
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| std::io::Error::other("original identity is missing"))?;
        if !backup.identity.matches(&backup.path)?
            || !original.matches(&backup.path.join("body"))?
        {
            return Err(std::io::Error::other(
                "old backup changed; retaining recovery paths",
            ));
        }
        Ok(())
    }

    fn rollback(&mut self) -> std::io::Result<()> {
        if self.state == PublicationState::RolledBack {
            return Ok(());
        }
        if self.state == PublicationState::Finalized {
            return Err(std::io::Error::other(
                "already finalized; rollback is no longer safe",
            ));
        }
        #[cfg(test)]
        if self.faults.rollback {
            return Err(std::io::Error::other("injected rollback failure"));
        }
        if matches!(
            self.state,
            PublicationState::Published | PublicationState::OldMoved
        ) && self.original.is_some()
        {
            self.check_backup()?;
        }
        if self.state == PublicationState::Published {
            if !self.staging.identity.matches(&self.destination)? {
                return Err(std::io::Error::other(
                    "published destination changed; refusing rollback deletion",
                ));
            }
            fs::remove_dir_all(&self.destination)?;
            // A failed restore can be retried without trying to delete an absent body.
            self.state = if self.original.is_some() {
                PublicationState::OldMoved
            } else {
                PublicationState::Prepared
            };
        }
        if self.state == PublicationState::OldMoved {
            require_absent(&self.destination)?;
            self.check_backup()?;
            let old = self
                .backup_path()
                .ok_or_else(|| std::io::Error::other("backup path is missing"))?;
            fs::rename(old, &self.destination)?;
            self.state = PublicationState::Prepared;
        }
        if self.staging.identity.matches(&self.staging.path)? {
            self.staging.remove()?;
        }
        if let Some(backup) = &self.backup {
            if !backup.identity.matches(&backup.path)? {
                return Err(std::io::Error::other(
                    "backup container changed; refusing cleanup",
                ));
            }
            fs::remove_dir(&backup.path)?;
        }
        self.state = PublicationState::RolledBack;
        Ok(())
    }
}

impl Drop for SkillInstallReceipt {
    fn drop(&mut self) {
        // A failed rollback or an unfinalized publication may have the only old
        // copy in backup/body. Never recursively remove that backup from Drop.
        if self
            .staging
            .identity
            .matches(&self.staging.path)
            .unwrap_or(false)
        {
            let _ = self.staging.remove();
        }
        if let Some(backup) = &self.backup {
            if backup.identity.matches(&backup.path).unwrap_or(false) {
                let _ = fs::remove_dir(&backup.path); // empty containers only
            }
        }
    }
}

pub(crate) fn validate_managed_destination(
    skill_dir: &Path,
    destination: &Path,
) -> Result<(), SkillInstallError> {
    let prepare_error = |path: &Path, message: String| SkillInstallError::Prepare {
        path: path.display().to_string(),
        message,
    };
    let root = skill_dir
        .canonicalize()
        .map_err(|error| prepare_error(skill_dir, error.to_string()))?;
    if !root.is_dir() {
        return Err(prepare_error(
            skill_dir,
            "managed skill store must be a directory".into(),
        ));
    }
    // Strip trailing separators/dots so metadata cannot follow a body symlink via `link/`.
    let destination = std::path::absolute(destination)
        .map_err(|error| prepare_error(destination, error.to_string()))?
        .components()
        .collect::<PathBuf>();
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(prepare_error(
                &destination,
                "managed body must be a directory, not a symlink or other file".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(prepare_error(&destination, error.to_string())),
    }

    let resolved = resolve_managed_directory(&destination)?;
    if resolved == root || !resolved.starts_with(&root) {
        return Err(prepare_error(
            &destination,
            "managed body must be strictly inside the configured skill store".into(),
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        prepare_error(
            &destination,
            "managed body must have a parent directory".into(),
        )
    })?;
    if !resolve_managed_directory(parent)?.starts_with(&root) {
        return Err(prepare_error(
            parent,
            "managed body parent escapes the configured skill store".into(),
        ));
    }
    Ok(())
}

/// Resolve through the nearest existing directory without creating missing namespaces.
/// symlink_metadata distinguishes a dangling alias from a genuinely missing directory.
fn resolve_managed_directory(path: &Path) -> Result<PathBuf, SkillInstallError> {
    let prepare_error = |message: String| SkillInstallError::Prepare {
        path: path.display().to_string(),
        message,
    };
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => {
                let mut resolved = existing
                    .canonicalize()
                    .map_err(|error| prepare_error(error.to_string()))?;
                if !resolved.is_dir() {
                    return Err(prepare_error(
                        "managed body parent must be a directory".into(),
                    ));
                }
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = existing.file_name().ok_or_else(|| {
                    prepare_error("missing managed path must not contain parent traversal".into())
                })?;
                missing.push(component);
                existing = existing
                    .parent()
                    .ok_or_else(|| prepare_error("managed path has no existing parent".into()))?;
            }
            Err(error) => return Err(prepare_error(error.to_string())),
        }
    }
}

fn install_to_staging(
    request: &SkillInstallRequest,
    staging: &Path,
) -> Result<InstalledSkillSource, SkillInstallError> {
    match &request.source {
        InstallSource::Local(path) => {
            if !path.exists() {
                return Err(SkillInstallError::MissingSourcePath {
                    path: path.display().to_string(),
                });
            }
            copy_package_contents(path, staging, request.include.as_ref())?;
            Ok(InstalledSkillSource {
                label: path.display().to_string(),
                resolved_source: request.source.clone(),
                warnings: Vec::new(),
            })
        }
        InstallSource::Git(git_source) => {
            install_git_to_staging(git_source, staging, request.include.as_ref())
        }
    }
}

fn install_git_to_staging(
    git_source: &GitInstallSource,
    staging: &Path,
    include: Option<&PackageFilter>,
) -> Result<InstalledSkillSource, SkillInstallError> {
    validate_git_subpath(&git_source.path)?;
    let clone_dir = staging.join(".repo");
    let git = GitClient;
    git.clone_checkout(git_source, &clone_dir)
        .map_err(skill_install_git_error)?;
    let source_path = safe_git_source_path(&clone_dir, &git_source.path)?;
    copy_package_contents(&source_path, staging, include)?;
    let rev = git
        .resolve_head(&clone_dir, &git_source.url)
        .map_err(skill_install_git_error)?;
    remove_dir(&clone_dir)?;
    let resolved_source = InstallSource::Git(GitInstallSource {
        url: git_source.url.clone(),
        reference: Some(rev.clone()),
        path: git_source.path.clone(),
    });
    Ok(InstalledSkillSource {
        label: format!("{}#{}:{}", git_source.url, rev, git_source.path.display()),
        resolved_source,
        warnings: Vec::new(),
    })
}

fn validate_git_subpath(path: &Path) -> Result<(), SkillInstallError> {
    let is_safe = !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::CurDir | Component::Normal(_)));
    if is_safe {
        Ok(())
    } else {
        Err(SkillInstallError::InvalidGitSubpath {
            path: path.display().to_string(),
            message: "path must be relative and must not contain '..'".to_owned(),
        })
    }
}

fn safe_git_source_path(clone_dir: &Path, subpath: &Path) -> Result<PathBuf, SkillInstallError> {
    let source_path = clone_dir.join(subpath);
    if !source_path.exists() {
        return Err(SkillInstallError::MissingSourcePath {
            path: source_path.display().to_string(),
        });
    }

    let canonical_clone = clone_dir
        .canonicalize()
        .map_err(|error| SkillInstallError::Prepare {
            path: clone_dir.display().to_string(),
            message: error.to_string(),
        })?;
    let canonical_source =
        source_path
            .canonicalize()
            .map_err(|error| SkillInstallError::Prepare {
                path: source_path.display().to_string(),
                message: error.to_string(),
            })?;
    if !canonical_source.starts_with(&canonical_clone) {
        return Err(SkillInstallError::InvalidGitSubpath {
            path: subpath.display().to_string(),
            message: "resolved path escapes cloned repository".to_owned(),
        });
    }

    Ok(canonical_source)
}

fn skill_install_git_error(error: GitCommandError) -> SkillInstallError {
    SkillInstallError::Git {
        repo: error.repo,
        message: error.message,
    }
}

fn validate_skill_package(path: &Path) -> Result<(), SkillInstallError> {
    let skill_md = path.join("SKILL.md");
    if !skill_md.exists() {
        return Err(SkillInstallError::InvalidSkillPackage {
            path: path.display().to_string(),
            message: "SKILL.md is missing".to_owned(),
        });
    }
    if !skill_md.is_file() {
        return Err(SkillInstallError::InvalidSkillPackage {
            path: skill_md.display().to_string(),
            message: "SKILL.md must be a file".to_owned(),
        });
    }

    let content = fs::read_to_string(&skill_md).map_err(|error| SkillInstallError::Prepare {
        path: skill_md.display().to_string(),
        message: error.to_string(),
    })?;
    parse_skill_manifest(&content).map_err(|error| SkillInstallError::InvalidSkillPackage {
        path: skill_md.display().to_string(),
        message: error.to_string(),
    })?;
    Ok(())
}

const PROTECTED_DIRS: &[&str] = &[".git", ".sksync", "node_modules"];

fn copy_package_contents(
    from: &Path,
    to: &Path,
    include: Option<&PackageFilter>,
) -> Result<(), SkillInstallError> {
    match include {
        None => copy_dir_contents(from, to),
        Some(filter) => copy_filtered_contents(from, to, filter),
    }
}

fn copy_dir_contents(from: &Path, to: &Path) -> Result<(), SkillInstallError> {
    for entry in fs::read_dir(from).map_err(|error| SkillInstallError::Copy {
        from: from.display().to_string(),
        to: to.display().to_string(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| SkillInstallError::Copy {
            from: from.display().to_string(),
            to: to.display().to_string(),
            message: error.to_string(),
        })?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| SkillInstallError::Copy {
            from: source_path.display().to_string(),
            to: target_path.display().to_string(),
            message: error.to_string(),
        })?;
        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| SkillInstallError::Prepare {
                path: target_path.display().to_string(),
                message: error.to_string(),
            })?;
            copy_dir_contents(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).map_err(|error| SkillInstallError::Copy {
                from: source_path.display().to_string(),
                to: target_path.display().to_string(),
                message: error.to_string(),
            })?;
        }
    }
    Ok(())
}

fn copy_filtered_contents(
    from: &Path,
    to: &Path,
    filter: &PackageFilter,
) -> Result<(), SkillInstallError> {
    let canonical_root = from
        .canonicalize()
        .map_err(|error| SkillInstallError::Prepare {
            path: from.display().to_string(),
            message: error.to_string(),
        })?;
    for pattern in filter.patterns() {
        let matches = include_matches(from, pattern)?;
        if matches.is_empty() {
            return Err(SkillInstallError::InvalidSkillPackage {
                path: from.display().to_string(),
                message: format!("include pattern matched no files: {pattern}"),
            });
        }
        for source_path in matches {
            copy_filtered_path(&canonical_root, &source_path, to)?;
        }
    }
    Ok(())
}

fn include_matches(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, SkillInstallError> {
    if !contains_glob(pattern) {
        let path = root.join(pattern);
        if path.exists() && !has_protected_component(path.strip_prefix(root).unwrap_or(&path)) {
            return Ok(vec![path]);
        }
        return Ok(Vec::new());
    }

    let mut matches = Vec::new();
    collect_matching_files(root, root, pattern, &mut matches)?;
    Ok(matches)
}

fn contains_glob(pattern: &str) -> bool {
    pattern.contains('*')
}

fn collect_matching_files(
    root: &Path,
    current: &Path,
    pattern: &str,
    matches: &mut Vec<PathBuf>,
) -> Result<(), SkillInstallError> {
    for entry in fs::read_dir(current).map_err(|error| SkillInstallError::Copy {
        from: current.display().to_string(),
        to: root.display().to_string(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| SkillInstallError::Copy {
            from: current.display().to_string(),
            to: root.display().to_string(),
            message: error.to_string(),
        })?;
        let source_path = entry.path();
        let relative = source_path.strip_prefix(root).unwrap_or(&source_path);
        if has_protected_component(relative) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| SkillInstallError::Copy {
            from: source_path.display().to_string(),
            to: root.display().to_string(),
            message: error.to_string(),
        })?;
        if file_type.is_dir() {
            collect_matching_files(root, &source_path, pattern, matches)?;
        } else if file_type.is_file() && glob_matches(relative, pattern) {
            matches.push(source_path);
        }
    }
    Ok(())
}

fn glob_matches(relative: &Path, pattern: &str) -> bool {
    if pattern == "**" {
        return true;
    }
    let relative = relative.to_string_lossy().replace('\\', "/");
    if let Some(prefix) = pattern.strip_suffix("/**") {
        let prefix = prefix.trim_end_matches('/');
        return relative == prefix || relative.starts_with(&format!("{prefix}/"));
    }
    let path_parts = relative.split('/').collect::<Vec<_>>();
    let pattern_parts = pattern.split('/').collect::<Vec<_>>();
    path_parts.len() == pattern_parts.len()
        && path_parts
            .iter()
            .zip(pattern_parts.iter())
            .all(|(path, pattern)| segment_matches(path, pattern))
}

fn segment_matches(value: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return value == pattern;
    };
    value.starts_with(prefix) && value.ends_with(suffix)
}

fn copy_filtered_path(
    root: &Path,
    source_path: &Path,
    target_root: &Path,
) -> Result<(), SkillInstallError> {
    let canonical_source =
        source_path
            .canonicalize()
            .map_err(|error| SkillInstallError::Prepare {
                path: source_path.display().to_string(),
                message: error.to_string(),
            })?;
    if !canonical_source.starts_with(root) {
        return Err(SkillInstallError::Copy {
            from: source_path.display().to_string(),
            to: target_root.display().to_string(),
            message: "include path escapes package root".to_owned(),
        });
    }
    let relative =
        canonical_source
            .strip_prefix(root)
            .map_err(|error| SkillInstallError::Copy {
                from: canonical_source.display().to_string(),
                to: target_root.display().to_string(),
                message: error.to_string(),
            })?;
    if has_protected_component(relative) {
        return Ok(());
    }
    let target_path = target_root.join(relative);
    if canonical_source.is_dir() {
        fs::create_dir_all(&target_path).map_err(|error| SkillInstallError::Prepare {
            path: target_path.display().to_string(),
            message: error.to_string(),
        })?;
        copy_filtered_directory(root, &canonical_source, target_root)?;
    } else if canonical_source.is_file() {
        copy_file(&canonical_source, &target_path)?;
    }
    Ok(())
}

fn copy_filtered_directory(
    root: &Path,
    source: &Path,
    target_root: &Path,
) -> Result<(), SkillInstallError> {
    for entry in fs::read_dir(source).map_err(|error| SkillInstallError::Copy {
        from: source.display().to_string(),
        to: target_root.display().to_string(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| SkillInstallError::Copy {
            from: source.display().to_string(),
            to: target_root.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if has_protected_component(relative) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| SkillInstallError::Copy {
            from: path.display().to_string(),
            to: target_root.display().to_string(),
            message: error.to_string(),
        })?;
        let target = target_root.join(relative);
        if file_type.is_dir() {
            fs::create_dir_all(&target).map_err(|error| SkillInstallError::Prepare {
                path: target.display().to_string(),
                message: error.to_string(),
            })?;
            copy_filtered_directory(root, &path, target_root)?;
        } else if file_type.is_file() {
            copy_file(&path, &target)?;
        }
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> Result<(), SkillInstallError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| SkillInstallError::Prepare {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
    }
    fs::copy(source, target).map_err(|error| SkillInstallError::Copy {
        from: source.display().to_string(),
        to: target.display().to_string(),
        message: error.to_string(),
    })?;
    Ok(())
}

fn has_protected_component(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|part| PROTECTED_DIRS.contains(&part))
    })
}

fn remove_dir(path: &Path) -> Result<(), SkillInstallError> {
    fs::remove_dir_all(path).map_err(|error| SkillInstallError::Prepare {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{validate_managed_destination, FileSystemSkillInstaller};
    use crate::application::ports::{SkillInstallError, SkillInstallRequest, SkillInstaller};
    use crate::domain::package_filter::PackageFilter;
    use crate::domain::source::{GitInstallSource, InstallSource};
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn update_destination_probe_allocator_excludes_all_missing_prefix_aliases() {
        use std::fs;
        if std::env::var_os("SKSYNC_TEST_DESTINATION_PROBE_ALLOCATOR").is_none() {
            let home = tempfile::tempdir().unwrap();
            let config = home.path().join(".config");
            fs::create_dir(&config).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .env("HOME", home.path()).env("USERPROFILE", home.path()).env("XDG_CONFIG_HOME", &config)
                .env("SKSYNC_TEST_DESTINATION_PROBE_ALLOCATOR", "1")
                .args(["--exact", "infrastructure::install::tests::update_destination_probe_allocator_excludes_all_missing_prefix_aliases", "--nocapture"])
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let excluded = [".SKSYNC", ".sKsync", ".ſksync", "legitimate-user-prefix"].map(|prefix| {
            temp.path().join(format!(
                "{prefix}-destination-probe-é-{}-0",
                std::process::id()
            ))
        });
        let paths = excluded
            .iter()
            .map(|path| path.as_path())
            .collect::<Vec<_>>();
        let owned =
            super::OwnedDirectory::create_excluding(temp.path(), "destination-probe-é", &paths)
                .unwrap();
        assert!(!owned
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with("-0"));
        assert!(
            excluded.iter().all(|path| !path.exists()),
            "probe allocation created a selected prefix alias"
        );
        owned.remove().unwrap();
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn update_destination_probe_cleanup_failure_reports_retained_path_and_original_overlap() {
        use std::fs;
        for overlaps in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("skills");
            fs::create_dir(&root).unwrap();
            let first = crate::domain::skill::SkillName::new("first").unwrap();
            let second = crate::domain::skill::SkillName::new("second").unwrap();
            let left = root.join("missing");
            let right = if overlaps {
                left.join("child")
            } else {
                root.join("other")
            };
            let error = super::validate_update_destination_aliases_inner(
                &root,
                &[(&first, &left), (&second, &right)],
                true,
            )
            .unwrap_err();
            let message = error.to_string();
            assert!(message.contains("injected alias probe cleanup failure"));
            assert_eq!(
                message.contains("dependency destinations overlap"),
                overlaps
            );
            let SkillInstallError::Prepare { path, .. } = error else {
                panic!("missing recovery path")
            };
            assert!(Path::new(&path).is_dir());
            assert!(message.contains(&path));
            assert!(!left.exists() && !right.exists());
            assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
            eprintln!("injected alias probe cleanup retained temporary owned root: {path}");
        }
    }

    #[test]
    fn update_destination_probe_reports_existing_parent_even_when_listed_last() {
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        fs::create_dir(&parent).unwrap();
        fs::write(parent.join("SKILL.md"), "old body").unwrap();
        let first = crate::domain::skill::SkillName::new("first").unwrap();
        let second = crate::domain::skill::SkillName::new("second").unwrap();
        let last = crate::domain::skill::SkillName::new("parent").unwrap();
        let left = parent.join("first");
        let right = parent.join("second");
        let error = super::validate_update_destination_aliases(
            temp.path(),
            &[(&first, &left), (&second, &right), (&last, &parent)],
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("'parent'") && error.contains("'first'"),
            "{error}"
        );
        assert!(!left.exists() && !right.exists());
        assert_eq!(fs::read(parent.join("SKILL.md")).unwrap(), b"old body");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    }

    #[test]
    fn update_destination_probe_refuses_cleanup_of_replaced_unmanaged_directory() {
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let owned =
            super::OwnedDirectory::create_excluding(temp.path(), "destination-probe-é", &[])
                .unwrap();
        let retained = temp.path().join("retained-original-probe");
        fs::rename(&owned.path, &retained).unwrap();
        fs::create_dir(&owned.path).unwrap();
        fs::write(owned.path.join("unmanaged"), "must remain").unwrap();
        assert!(owned.remove().is_err());
        assert_eq!(
            fs::read(owned.path.join("unmanaged")).unwrap(),
            b"must remain"
        );
        assert!(retained.is_dir());
    }

    #[test]
    fn prepared_install_skips_final_destination_as_staging_candidate() {
        use crate::application::ports::PreparedSkillInstaller;
        // A fresh exact-test process makes the allocator's first sequence zero
        // without sharing or mutating its counter with parallel installer tests.
        if std::env::var_os("SKSYNC_TEST_STAGING_CANDIDATE").is_none() {
            let home = tempfile::tempdir().unwrap();
            let config = home.path().join(".config");
            std::fs::create_dir(&config).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .env("HOME", home.path())
                .env("USERPROFILE", home.path())
                .env("XDG_CONFIG_HOME", &config)
                .env("SKSYNC_TEST_STAGING_CANDIDATE", "1")
                .arg("--exact")
                .arg("infrastructure::install::tests::prepared_install_skips_final_destination_as_staging_candidate")
                .arg("--nocapture")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "collision child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let remote = temp.path().join("remote");
        std::fs::create_dir(&remote).unwrap();
        let content = skill_md("review", "New");
        std::fs::write(remote.join("SKILL.md"), &content).unwrap();
        let name = format!(".sksync-staging-{}-0", std::process::id());
        assert!(crate::domain::skill::SkillName::new(&name).is_ok());
        let destination = root.join(&name);
        let mut prepared = FileSystemSkillInstaller
            .prepare_skill(
                &request(InstallSource::Local(remote), &root),
                &destination,
                &name,
            )
            .unwrap();
        assert!(
            !destination.exists(),
            "preparation created the absent live body as staging"
        );
        assert_ne!(prepared.receipt.staging.path, prepared.receipt.destination);
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            content
        );
        FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap();
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn prepared_install_skips_case_alias_of_final_destination_as_staging_candidate() {
        use crate::application::ports::PreparedSkillInstaller;
        // A fresh exact-test process makes the allocator's first sequence zero
        // without sharing or mutating its counter with parallel installer tests.
        if std::env::var_os("SKSYNC_TEST_STAGING_CANDIDATE").is_none() {
            let home = tempfile::tempdir().unwrap();
            let config = home.path().join(".config");
            std::fs::create_dir(&config).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .env("HOME", home.path())
                .env("USERPROFILE", home.path())
                .env("XDG_CONFIG_HOME", &config)
                .env("SKSYNC_TEST_STAGING_CANDIDATE", "1")
                .arg("--exact")
                .arg("infrastructure::install::tests::prepared_install_skips_case_alias_of_final_destination_as_staging_candidate")
                .arg("--nocapture")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "collision child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let remote = temp.path().join("remote");
        std::fs::create_dir(&remote).unwrap();
        let content = skill_md("review", "New");
        std::fs::write(remote.join("SKILL.md"), &content).unwrap();
        let probe = temp.path().join("CaseProbe");
        std::fs::create_dir(&probe).unwrap();
        eprintln!(
            "temporary filesystem case-insensitive alias: {}",
            temp.path().join("cASEpROBE").is_dir()
        );
        let name = format!(".SKSYNC-STAGING-{}-0", std::process::id());
        assert!(crate::domain::skill::SkillName::new(&name).is_ok());
        let destination = root.join(&name);
        let mut prepared = FileSystemSkillInstaller
            .prepare_skill(
                &request(InstallSource::Local(remote), &root),
                &destination,
                &name,
            )
            .unwrap();
        assert!(
            !destination.exists(),
            "preparation created the absent live body through a case alias of staging"
        );
        assert!(
            !prepared
                .receipt
                .staging
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .eq_ignore_ascii_case(&name),
            "allocation must conservatively exclude final-name case aliases"
        );
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            content
        );
        FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap();
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn prepared_install_skips_unicode_alias_of_final_destination_as_staging_candidate() {
        use crate::application::ports::PreparedSkillInstaller;
        // A fresh exact-test process makes the allocator's first sequence zero
        // without sharing or mutating its counter with parallel installer tests.
        if std::env::var_os("SKSYNC_TEST_STAGING_CANDIDATE").is_none() {
            let home = tempfile::tempdir().unwrap();
            let config = home.path().join(".config");
            std::fs::create_dir(&config).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .env("HOME", home.path())
                .env("USERPROFILE", home.path())
                .env("XDG_CONFIG_HOME", &config)
                .env("SKSYNC_TEST_STAGING_CANDIDATE", "1")
                .arg("--exact")
                .arg("infrastructure::install::tests::prepared_install_skips_unicode_alias_of_final_destination_as_staging_candidate")
                .arg("--nocapture")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "collision child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let remote = temp.path().join("remote");
        std::fs::create_dir(&remote).unwrap();
        let content = skill_md("review", "New");
        std::fs::write(remote.join("SKILL.md"), &content).unwrap();
        let probe = temp.path().join("KelvinProbe");
        std::fs::create_dir(&probe).unwrap();
        eprintln!(
            "temporary filesystem Kelvin-sign alias: {}",
            temp.path().join("KelvinProbe").is_dir()
        );
        let name = format!(".sKsync-staging-{}-0", std::process::id());
        assert!(crate::domain::skill::SkillName::new(&name).is_ok());
        let destination = root.join(&name);
        let mut prepared = FileSystemSkillInstaller
            .prepare_skill(
                &request(InstallSource::Local(remote), &root),
                &destination,
                &name,
            )
            .unwrap();
        assert!(
            !destination.exists(),
            "preparation created the absent live body through a Unicode alias of staging"
        );
        assert!(
            !prepared
                .receipt
                .staging
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-0"),
            "allocation must exclude the matching sequence suffix independently of prefix"
        );
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            content
        );
        FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap();
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn prepared_install_skips_long_s_alias_of_final_destination_as_staging_candidate() {
        use crate::application::ports::PreparedSkillInstaller;
        // A fresh exact-test process makes the allocator's first sequence zero
        // without sharing or mutating its counter with parallel installer tests.
        if std::env::var_os("SKSYNC_TEST_STAGING_CANDIDATE").is_none() {
            let home = tempfile::tempdir().unwrap();
            let config = home.path().join(".config");
            std::fs::create_dir(&config).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .env("HOME", home.path())
                .env("USERPROFILE", home.path())
                .env("XDG_CONFIG_HOME", &config)
                .env("SKSYNC_TEST_STAGING_CANDIDATE", "1")
                .arg("--exact")
                .arg("infrastructure::install::tests::prepared_install_skips_long_s_alias_of_final_destination_as_staging_candidate")
                .arg("--nocapture")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "collision child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let remote = temp.path().join("remote");
        std::fs::create_dir(&remote).unwrap();
        let content = skill_md("review", "New");
        std::fs::write(remote.join("SKILL.md"), &content).unwrap();
        let probe = temp.path().join("LongSProbe");
        std::fs::create_dir(&probe).unwrap();
        eprintln!(
            "temporary filesystem long-s alias: {}",
            temp.path().join("LongſProbe").is_dir()
        );
        let name = format!(".ſksync-staging-{}-0", std::process::id());
        assert!(crate::domain::skill::SkillName::new(&name).is_ok());
        let destination = root.join(&name);
        let mut prepared = FileSystemSkillInstaller
            .prepare_skill(
                &request(InstallSource::Local(remote), &root),
                &destination,
                &name,
            )
            .unwrap();
        assert!(
            !destination.exists(),
            "preparation created the absent live body through a Unicode alias of staging"
        );
        assert!(
            !prepared
                .receipt
                .staging
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-0"),
            "allocation must exclude the matching sequence suffix independently of prefix"
        );
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            content
        );
        FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap();
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn prepared_install_skips_matching_sequence_suffix_with_unrelated_prefix() {
        use crate::application::ports::PreparedSkillInstaller;
        // A fresh exact-test process makes the allocator's first sequence zero
        // without sharing or mutating its counter with parallel installer tests.
        if std::env::var_os("SKSYNC_TEST_STAGING_CANDIDATE").is_none() {
            let home = tempfile::tempdir().unwrap();
            let config = home.path().join(".config");
            std::fs::create_dir(&config).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .env("HOME", home.path())
                .env("USERPROFILE", home.path())
                .env("XDG_CONFIG_HOME", &config)
                .env("SKSYNC_TEST_STAGING_CANDIDATE", "1")
                .arg("--exact")
                .arg("infrastructure::install::tests::prepared_install_skips_matching_sequence_suffix_with_unrelated_prefix")
                .arg("--nocapture")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "collision child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let remote = temp.path().join("remote");
        std::fs::create_dir(&remote).unwrap();
        let content = skill_md("review", "New");
        std::fs::write(remote.join("SKILL.md"), &content).unwrap();
        let name = format!("unrelated-public-prefix-{}-0", std::process::id());
        assert!(crate::domain::skill::SkillName::new(&name).is_ok());
        let destination = root.join(&name);
        let mut prepared = FileSystemSkillInstaller
            .prepare_skill(
                &request(InstallSource::Local(remote), &root),
                &destination,
                &name,
            )
            .unwrap();
        assert!(
            !destination.exists(),
            "preparation must preserve destination absence"
        );
        assert!(
            !prepared
                .receipt
                .staging
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-0"),
            "allocation must exclude the matching sequence suffix independently of prefix"
        );
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            content
        );
        FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap();
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn prepared_hash_and_files_match_published_bodies_with_excluded_directory_names() {
        use crate::application::ports::PreparedSkillInstaller;
        use crate::domain::lockfile::LockedFile;
        for name in ["target", "node_modules", ".git"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("skills");
            let remote = temp.path().join("remote");
            std::fs::create_dir(&remote).unwrap();
            std::fs::write(remote.join("SKILL.md"), skill_md(name, "New")).unwrap();
            std::fs::write(remote.join("guide.md"), b"guide").unwrap();
            for descendant in ["target", "node_modules", ".git"] {
                std::fs::create_dir(remote.join(descendant)).unwrap();
                std::fs::write(remote.join(descendant).join("ignored"), b"ignored").unwrap();
            }
            let destination = root.join(name);
            let mut prepared = FileSystemSkillInstaller
                .prepare_skill(
                    &request(InstallSource::Local(remote), &root),
                    &destination,
                    name,
                )
                .unwrap();
            assert!(!destination.exists());
            assert_eq!(prepared.files.len(), 2);
            FileSystemSkillInstaller
                .publish_skill(&mut prepared.receipt)
                .unwrap();
            let published = crate::infrastructure::hash::hash_directory(&destination).unwrap();
            assert_eq!(
                prepared.hash, published.hash,
                "directory hash changed for {name}"
            );
            let files = published
                .files
                .into_iter()
                .map(|file| LockedFile {
                    path: file.path,
                    hash: file.hash,
                })
                .collect::<Vec<_>>();
            assert_eq!(prepared.files, files, "file list changed for {name}");
            assert!(FileSystemSkillInstaller
                .finalize_skill(&mut prepared.receipt)
                .is_empty());
        }
    }

    #[test]
    fn prepared_install_leaves_old_body_intact_and_hashes_filtered_staging() {
        use crate::application::ports::PreparedSkillInstaller;
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("skills/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("SKILL.md"), skill_md("review", "Old")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "New")).unwrap();
        std::fs::write(remote.join("ignored.txt"), b"ignored").unwrap();
        let prepared = FileSystemSkillInstaller
            .prepare_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::manifest_only(),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            skill_md("review", "Old")
        );
        assert_eq!(prepared.name.as_str(), "review");
        assert_eq!(prepared.destination, destination);
        assert_eq!(prepared.files.len(), 1);
        assert_eq!(prepared.files[0].path, Path::new("SKILL.md"));
    }

    fn prepared_fixture(
        existing: bool,
    ) -> (
        tempfile::TempDir,
        crate::application::ports::PreparedSkill<super::SkillInstallReceipt>,
    ) {
        use crate::application::ports::PreparedSkillInstaller;
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("skills/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "New")).unwrap();
        if existing {
            std::fs::create_dir_all(&destination).unwrap();
            std::fs::write(destination.join("SKILL.md"), skill_md("review", "Old")).unwrap();
        }
        let prepared = FileSystemSkillInstaller
            .prepare_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review",
            )
            .unwrap();
        (temp, prepared)
    }

    #[test]
    fn prepared_install_failure_after_old_move_rolls_back_old_bytes() {
        use crate::application::ports::PreparedSkillInstaller;
        let (_temp, mut prepared) = prepared_fixture(true);
        prepared.receipt.faults.publish_after_backup = true;
        assert!(FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .is_err());
        assert!(!prepared.destination.exists());
        FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(prepared.destination.join("SKILL.md")).unwrap(),
            skill_md("review", "Old")
        );
    }

    #[test]
    fn prepared_install_published_body_rolls_back_to_old_bytes_or_absence() {
        use crate::application::ports::PreparedSkillInstaller;
        for existing in [true, false] {
            let (_temp, mut prepared) = prepared_fixture(existing);
            FileSystemSkillInstaller
                .publish_skill(&mut prepared.receipt)
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(prepared.destination.join("SKILL.md")).unwrap(),
                skill_md("review", "New")
            );
            assert_eq!(
                prepared.hash,
                crate::infrastructure::hash::hash_directory(&prepared.destination)
                    .unwrap()
                    .hash
            );
            FileSystemSkillInstaller
                .rollback_skill(&mut prepared.receipt)
                .unwrap();
            FileSystemSkillInstaller
                .rollback_skill(&mut prepared.receipt)
                .unwrap();
            if existing {
                assert_eq!(
                    std::fs::read_to_string(prepared.destination.join("SKILL.md")).unwrap(),
                    skill_md("review", "Old")
                );
            } else {
                assert!(!prepared.destination.exists());
            }
        }
    }

    #[test]
    fn prepared_install_unpublished_rollback_and_drop_only_clean_staging() {
        use crate::application::ports::PreparedSkillInstaller;
        for existing in [true, false] {
            let (temp, mut prepared) = prepared_fixture(existing);
            let staging = prepared.receipt.staging.path.clone();
            FileSystemSkillInstaller
                .rollback_skill(&mut prepared.receipt)
                .unwrap();
            assert!(!staging.exists());
            assert_eq!(prepared.destination.exists(), existing);
            drop(prepared);
            assert_eq!(
                std::fs::read_dir(temp.path().join("skills"))
                    .unwrap()
                    .count(),
                usize::from(existing)
            );
            let (_temp, prepared) = prepared_fixture(existing);
            let staging = prepared.receipt.staging.path.clone();
            let destination = prepared.destination.clone();
            drop(prepared);
            assert!(!staging.exists());
            assert_eq!(destination.exists(), existing);
        }
    }

    #[test]
    fn prepared_install_rollback_failure_retains_backup_through_drop() {
        use crate::application::ports::PreparedSkillInstaller;
        let (_temp, mut prepared) = prepared_fixture(true);
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        prepared.receipt.faults.rollback = true;
        let failure = FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap_err();
        assert_eq!(failure.skill.as_str(), "review");
        let backup = failure.retained_backup.clone().unwrap();
        assert!(failure.to_string().contains(&backup.display().to_string()));
        drop(prepared);
        assert_eq!(
            std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
            skill_md("review", "Old")
        );
    }

    #[test]
    fn prepared_install_finalize_failure_is_warning_not_rollback() {
        use crate::application::ports::PreparedSkillInstaller;
        let (_temp, mut prepared) = prepared_fixture(true);
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        prepared.receipt.faults.finalize = true;
        let warnings = FileSystemSkillInstaller.finalize_skill(&mut prepared.receipt);
        assert_eq!(warnings.len(), 1);
        let retained = warnings[0].path.clone();
        assert_eq!(
            std::fs::read_to_string(prepared.destination.join("SKILL.md")).unwrap(),
            skill_md("review", "New")
        );
        assert!(FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .is_err());
        drop(prepared);
        assert!(retained.is_dir());
    }

    #[test]
    fn prepared_install_finalize_removes_only_owned_backups() {
        use crate::application::ports::PreparedSkillInstaller;
        let (temp, mut prepared) = prepared_fixture(true);
        let foreign = temp.path().join("skills/.sksync-update-review-foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(foreign.join("keep"), b"foreign").unwrap();
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        assert!(FileSystemSkillInstaller
            .finalize_skill(&mut prepared.receipt)
            .is_empty());
        drop(prepared);
        assert_eq!(std::fs::read(foreign.join("keep")).unwrap(), b"foreign");
        assert_eq!(
            std::fs::read_dir(temp.path().join("skills"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn prepared_install_rejects_changed_destination_without_deleting_it() {
        use crate::application::ports::PreparedSkillInstaller;
        let (_temp, mut prepared) = prepared_fixture(true);
        let original = prepared.destination.with_extension("external");
        std::fs::rename(&prepared.destination, &original).unwrap();
        std::fs::create_dir(&prepared.destination).unwrap();
        std::fs::write(prepared.destination.join("keep"), b"foreign").unwrap();
        assert!(FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .is_err());
        FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read(prepared.destination.join("keep")).unwrap(),
            b"foreign"
        );
        assert_eq!(
            std::fs::read_to_string(original.join("SKILL.md")).unwrap(),
            skill_md("review", "Old")
        );
    }

    #[test]
    fn prepared_install_rollback_preserves_unexpected_file_and_old_copy() {
        use crate::application::ports::PreparedSkillInstaller;
        let (_temp, mut prepared) = prepared_fixture(true);
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        std::fs::rename(
            &prepared.destination,
            prepared.destination.with_extension("external"),
        )
        .unwrap();
        std::fs::write(&prepared.destination, b"foreign").unwrap();
        let failure = FileSystemSkillInstaller
            .rollback_skill(&mut prepared.receipt)
            .unwrap_err();
        let backup = failure.retained_backup.unwrap();
        drop(prepared);
        assert_eq!(
            std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
            skill_md("review", "Old")
        );
        assert_eq!(
            std::fs::read(backup.parent().unwrap().parent().unwrap().join("review")).unwrap(),
            b"foreign"
        );
    }

    #[test]
    fn prepared_install_invalid_package_preserves_old_body_and_cleans_staging() {
        use crate::application::ports::PreparedSkillInstaller;
        let (temp, prepared) = prepared_fixture(true);
        drop(prepared);
        let remote = temp.path().join("remote");
        std::fs::write(remote.join("SKILL.md"), b"invalid").unwrap();
        let destination = temp.path().join("skills/review");
        assert!(FileSystemSkillInstaller
            .prepare_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review"
            )
            .is_err());
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            skill_md("review", "Old")
        );
        assert_eq!(
            std::fs::read_dir(temp.path().join("skills"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn legacy_install_wrapper_reports_original_and_retained_recovery_path() {
        let (_temp, mut prepared) = prepared_fixture(true);
        prepared.receipt.faults.publish_after_backup = true;
        prepared.receipt.faults.rollback = true;
        let error = super::install_prepared(prepared).unwrap_err();
        assert!(matches!(error, SkillInstallError::RollbackFailed { .. }));
        let message = error.to_string();
        assert!(message.contains("injected publication failure"));
        assert!(message.contains("injected rollback failure"));
        assert!(message.contains(".sksync-backup-"));
    }

    #[test]
    fn prepared_install_sibling_staging_is_exclusive_and_preserves_legacy_leftovers() {
        use crate::application::ports::PreparedSkillInstaller;
        let (temp, first) = prepared_fixture(true);
        let legacy = temp.path().join(format!(
            "skills/.sksync-update-review-{}",
            std::process::id()
        ));
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("keep"), b"foreign").unwrap();
        let destination = temp.path().join("skills/review");
        let second = FileSystemSkillInstaller
            .prepare_skill(
                &request(
                    InstallSource::Local(temp.path().join("remote")),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();
        assert_ne!(first.receipt.staging.path, second.receipt.staging.path);
        let physical_parent = destination.parent().unwrap().canonicalize().unwrap();
        assert_eq!(
            first.receipt.staging.path.parent(),
            Some(physical_parent.as_path())
        );
        assert_eq!(
            second.receipt.staging.path.parent(),
            Some(physical_parent.as_path())
        );
        drop(first);
        drop(second);
        assert_eq!(std::fs::read(legacy.join("keep")).unwrap(), b"foreign");
    }

    #[test]
    fn prepared_install_drop_never_deletes_replaced_staging_symlink() {
        let (_temp, prepared) = prepared_fixture(true);
        let staging = prepared.receipt.staging.path.clone();
        let moved = staging.with_extension("external");
        std::fs::rename(&staging, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &staging).unwrap();
        drop(prepared);
        assert_eq!(std::fs::read_link(&staging).unwrap(), moved);
        assert_eq!(
            std::fs::read_to_string(staging.join("SKILL.md")).unwrap(),
            skill_md("review", "New")
        );
    }

    #[test]
    fn legacy_install_wrapper_returns_cleanup_warnings_after_success() {
        let (_temp, mut prepared) = prepared_fixture(true);
        let destination = prepared.destination.clone();
        prepared.receipt.faults.finalize = true;
        let installed = super::install_prepared(prepared).unwrap();
        assert_eq!(installed.warnings.len(), 1);
        assert!(installed.warnings[0].path.join("body/SKILL.md").exists());
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            skill_md("review", "New")
        );
    }

    #[test]
    fn legacy_install_wrapper_restores_old_bytes_or_absence_on_publication_error() {
        for existing in [true, false] {
            let (_temp, mut prepared) = prepared_fixture(existing);
            let destination = prepared.destination.clone();
            prepared.receipt.faults.publish_after_backup = true;
            let error = super::install_prepared(prepared).unwrap_err();
            assert!(!matches!(error, SkillInstallError::RollbackFailed { .. }));
            if existing {
                assert_eq!(
                    std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
                    skill_md("review", "Old")
                );
            } else {
                assert!(!destination.exists());
            }
            assert_eq!(
                std::fs::read_dir(destination.parent().unwrap())
                    .unwrap()
                    .count(),
                usize::from(existing)
            );
        }
    }

    #[test]
    fn prepared_git_install_metadata_records_actual_ref_and_filtered_content() {
        use crate::application::ports::PreparedSkillInstaller;
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        create_git_skill_repo(&remote, &skill_md("review", "Old"));
        let rev = git_output(&remote, &["rev-parse", "HEAD"]);
        std::fs::write(
            remote.join("skills/review/SKILL.md"),
            skill_md("review", "New"),
        )
        .unwrap();
        git(&remote, &["add", "."]);
        git(&remote, &["commit", "-m", "new"]);
        let root = temp.path().join("store");
        let destination = root.join("github/repo/review");
        let source = InstallSource::Git(GitInstallSource {
            url: remote.display().to_string(),
            reference: Some(rev.clone()),
            path: "skills/review".into(),
        });
        let mut prepared = FileSystemSkillInstaller
            .prepare_skill(
                &filtered_request(source.clone(), PackageFilter::manifest_only(), &root),
                &destination,
                "review",
            )
            .unwrap();
        assert!(!destination.exists());
        assert_eq!(prepared.destination, destination);
        assert_eq!(prepared.installed.resolved_source, source);
        assert_eq!(prepared.files[0].path, Path::new("SKILL.md"));
        FileSystemSkillInstaller
            .publish_skill(&mut prepared.receipt)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            skill_md("review", "Old")
        );
        let hashes = crate::infrastructure::hash::hash_directory(&destination).unwrap();
        assert_eq!(prepared.hash, hashes.hash);
        assert_eq!(prepared.files[0].hash, hashes.files[0].hash);
        assert!(FileSystemSkillInstaller
            .finalize_skill(&mut prepared.receipt)
            .is_empty());
    }

    #[test]
    fn legacy_install_rejects_unmanaged_body_files_symlinks_and_escaping_parent() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        std::fs::create_dir(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "New")).unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir(&root).unwrap();
        let destination = root.join("review");
        let request = request(InstallSource::Local(remote), &root);
        std::fs::write(&destination, b"foreign").unwrap();
        assert!(FileSystemSkillInstaller
            .install_skill(&request, &destination, "review")
            .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"foreign");
        std::fs::remove_file(&destination).unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &destination).unwrap();
        assert!(FileSystemSkillInstaller
            .install_skill(&request, &destination, "review")
            .is_err());
        assert!(FileSystemSkillInstaller
            .install_skill(&request, &destination.join("missing/review"), "review")
            .is_err());
        assert_eq!(std::fs::read_link(&destination).unwrap(), outside);
        assert!(!outside.join("missing").exists());
    }

    #[test]
    fn managed_destination_accepts_flat_and_missing_namespaced_bodies() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir(&root).unwrap();
        let flat = root.join("review");
        std::fs::create_dir(&flat).unwrap();
        std::fs::write(flat.join("SKILL.md"), b"old").unwrap();
        validate_managed_destination(&root, &flat).unwrap();
        let namespaced = root.join("local/source/.review");
        validate_managed_destination(&root, &namespaced).unwrap();
        assert!(!root.join("local").exists());
        assert_eq!(std::fs::read(flat.join("SKILL.md")).unwrap(), b"old");
    }

    #[test]
    fn managed_destination_rejects_store_root_and_outside_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("namespace")).unwrap();
        std::fs::create_dir(temp.path().join("skills-other")).unwrap();
        for destination in [
            root.clone(),
            root.join("."),
            root.join("namespace/.."),
            root.join("../review"),
            temp.path().join("skills-other/review"),
            root.join("missing/../../review"),
        ] {
            assert!(
                validate_managed_destination(&root, &destination).is_err(),
                "accepted {}",
                destination.display()
            );
        }
    }

    #[test]
    fn managed_destination_rejects_missing_or_non_directory_store_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        assert!(validate_managed_destination(&root, &root.join("review")).is_err());
        std::fs::write(&root, b"store file").unwrap();
        assert!(validate_managed_destination(&root, &root.join("review")).is_err());
        assert_eq!(std::fs::read(&root).unwrap(), b"store file");
    }

    #[test]
    fn managed_destination_rejects_regular_files_and_non_directory_parents() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let file = root.join("review");
        std::fs::write(&file, b"unmanaged").unwrap();
        assert!(validate_managed_destination(root, &file).is_err());
        assert!(validate_managed_destination(root, &file.join("child")).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"unmanaged");
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_existing_and_dangling_body_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(root.join("real")).unwrap();
        for (name, target) in [
            ("linked", root.join("real")),
            ("dangling", root.join("absent")),
        ] {
            let destination = root.join(name);
            symlink(&target, &destination).unwrap();
            for body in [
                destination.clone(),
                destination.join(""),
                destination.join("."),
            ] {
                assert!(
                    validate_managed_destination(&root, &body).is_err(),
                    "accepted body alias {}",
                    body.display()
                );
            }
            assert_eq!(std::fs::read_link(&destination).unwrap(), target);
        }
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_escaping_parent_aliases() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir_all(outside.join("existing")).unwrap();
        symlink(&outside, root.join("alias")).unwrap();
        for destination in [
            root.join("alias/existing"),
            root.join("alias/new/namespace/review"),
        ] {
            assert!(validate_managed_destination(&root, &destination).is_err());
        }
        assert!(!outside.join("new").exists());
        assert_eq!(std::fs::read_link(root.join("alias")).unwrap(), outside);
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_dangling_parent_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let alias = root.join("alias");
        let target = root.join("absent");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        assert!(validate_managed_destination(root, &alias.join("namespace/review")).is_err());
        assert_eq!(std::fs::read_link(alias).unwrap(), target);
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_accepts_authorized_store_symlink_and_internal_parent_alias() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let physical = temp.path().join("physical");
        let root = temp.path().join("store");
        std::fs::create_dir_all(physical.join("namespace/existing")).unwrap();
        symlink(&physical, &root).unwrap();
        symlink(physical.join("namespace"), physical.join("alias")).unwrap();
        validate_managed_destination(&root, &root.join("namespace/existing")).unwrap();
        validate_managed_destination(&root, &root.join("alias/missing/review")).unwrap();
        assert!(validate_managed_destination(&root, &root).is_err());
        assert!(!physical.join("namespace/missing").exists());
        assert_eq!(std::fs::read_link(&root).unwrap(), physical);
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_special_files() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("socket");
        let _socket = std::os::unix::net::UnixListener::bind(&destination).unwrap();
        assert!(validate_managed_destination(temp.path(), &destination).is_err());
        assert!(std::fs::symlink_metadata(&destination).is_ok());
    }

    #[test]
    fn local_dependency_is_copied_into_destination() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        let destination = temp.path().join("skills/review");

        FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review",
            )
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            skill_md("review", "Review helper")
        );
    }

    #[test]
    fn git_dependency_can_install_exact_commit_reference() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        create_git_skill_repo(&remote, &skill_md("review", "Review v1"));
        let rev = git_output(&remote, &["rev-parse", "HEAD"]);
        std::fs::write(
            remote.join("skills/review/SKILL.md"),
            skill_md("review", "Review v2"),
        )
        .unwrap();
        git(&remote, &["add", "."]);
        git(&remote, &["commit", "-m", "update review"]);
        let destination = temp.path().join("skills/review");

        FileSystemSkillInstaller
            .install_skill(
                &request(
                    InstallSource::Git(GitInstallSource {
                        url: remote.display().to_string(),
                        reference: Some(rev.clone()),
                        path: "skills/review".into(),
                    }),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            skill_md("review", "Review v1")
        );
    }

    #[test]
    fn git_install_rejects_parent_directory_subpath() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        create_git_skill_repo(&remote, &skill_md("review", "Review helper"));
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(
                &request(
                    InstallSource::Git(GitInstallSource {
                        url: remote.display().to_string(),
                        reference: None,
                        path: "../review".into(),
                    }),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .expect_err("parent directory git subpath should fail");

        assert!(matches!(error, SkillInstallError::InvalidGitSubpath { .. }));
        assert!(!destination.exists());
    }

    #[test]
    fn git_install_rejects_symlink_escape_subpath() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("SKILL.md"),
            skill_md("outside", "Outside skill"),
        )
        .unwrap();
        create_git_skill_repo(&remote, &skill_md("review", "Review helper"));
        std::os::unix::fs::symlink(&outside, remote.join("skills/escape")).unwrap();
        git(&remote, &["add", "."]);
        git(&remote, &["commit", "-m", "add escape symlink"]);
        let destination = temp.path().join("skills/escape");

        let error = FileSystemSkillInstaller
            .install_skill(
                &request(
                    InstallSource::Git(GitInstallSource {
                        url: remote.display().to_string(),
                        reference: None,
                        path: "skills/escape".into(),
                    }),
                    destination.parent().unwrap(),
                ),
                &destination,
                "escape",
            )
            .expect_err("symlink escape git subpath should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidGitSubpath { message, .. }
                if message == "resolved path escapes cloned repository"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_skill_md_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review",
            )
            .expect_err("missing SKILL.md should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md is missing"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_frontmatter_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), "# Review\n").unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review",
            )
            .expect_err("missing frontmatter should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md YAML frontmatter is missing"
        ));
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read_dir(destination.parent().unwrap())
                .unwrap()
                .count(),
            0,
            "failed preparation must remove its owned staging directory"
        );
    }

    #[test]
    fn install_fails_when_required_frontmatter_field_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(
            remote.join("SKILL.md"),
            "---\ndescription: Review helper\n---\n# Review\n",
        )
        .unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review",
            )
            .expect_err("missing name should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md frontmatter field 'name' is required"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_required_frontmatter_field_is_empty() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(
            remote.join("SKILL.md"),
            "---\nname: review\ndescription: '   '\n---\n# Review\n",
        )
        .unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review",
            )
            .expect_err("empty description should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md frontmatter field 'description' must not be empty"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_required_frontmatter_field_is_not_a_string() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(
            remote.join("SKILL.md"),
            "---\nname: review\ndescription: 123\n---\n# Review\n",
        )
        .unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(remote), destination.parent().unwrap()),
                &destination,
                "review",
            )
            .expect_err("non-string description should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md frontmatter field 'description' must be a string"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_with_manifest_only_copies_only_skill_md() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join("src")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("herdr", "Herdr helper")).unwrap();
        std::fs::write(remote.join("Cargo.toml"), "[package]\nname = \"herdr\"\n").unwrap();
        std::fs::write(remote.join("src/main.rs"), "fn main() {}\n").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::manifest_only(),
                    destination.parent().unwrap(),
                ),
                &destination,
                "herdr",
            )
            .unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(!destination.join("Cargo.toml").exists());
        assert!(!destination.join("src").exists());
    }

    #[test]
    fn install_with_directory_include_recursively_copies_references() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join("references/nested")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        std::fs::write(remote.join("references/nested/guide.md"), "guide").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["SKILL.md".into(), "references".into()]).unwrap(),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(destination.join("references/nested/guide.md").is_file());
    }

    #[test]
    fn install_with_glob_include_copies_matching_files() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join("assets")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        std::fs::write(remote.join("assets/logo.png"), "png").unwrap();
        std::fs::write(remote.join("assets/readme.txt"), "txt").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["SKILL.md".into(), "assets/*.png".into()]).unwrap(),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();

        assert!(destination.join("assets/logo.png").is_file());
        assert!(!destination.join("assets/readme.txt").exists());
    }

    #[test]
    fn include_pattern_must_match() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();

        let error = FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["missing".into()]).unwrap(),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("include pattern matched no files"));
    }

    #[test]
    fn protected_dirs_are_not_copied_by_filtered_installs() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join(".git/objects")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        std::fs::write(remote.join(".git/config"), "secret").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["SKILL.md".into(), "**".into()]).unwrap(),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(!destination.join(".git/config").exists());
    }

    #[test]
    fn staging_directory_is_removed_when_install_fails() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("skills/review");
        let error = FileSystemSkillInstaller
            .install_skill(
                &request(
                    InstallSource::Local(temp.path().join("missing")),
                    destination.parent().unwrap(),
                ),
                &destination,
                "review",
            )
            .expect_err("missing source should fail");

        assert!(matches!(error, SkillInstallError::MissingSourcePath { .. }));
        assert_eq!(
            std::fs::read_dir(destination.parent().unwrap())
                .unwrap()
                .count(),
            0,
            "failed preparation must remove its owned staging directory"
        );
    }

    fn request(source: InstallSource, managed_root: &Path) -> SkillInstallRequest {
        SkillInstallRequest {
            managed_root: managed_root.to_path_buf(),
            source,
            include: None,
        }
    }

    fn filtered_request(
        source: InstallSource,
        include: PackageFilter,
        managed_root: &Path,
    ) -> SkillInstallRequest {
        SkillInstallRequest {
            managed_root: managed_root.to_path_buf(),
            source,
            include: Some(include),
        }
    }

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n")
    }

    fn create_git_skill_repo(path: &Path, skill_content: &str) {
        std::fs::create_dir_all(path.join("skills/review")).unwrap();
        git(path, &["init"]);
        git(path, &["config", "user.email", "test@example.com"]);
        git(path, &["config", "user.name", "Test User"]);
        std::fs::write(path.join("skills/review/SKILL.md"), skill_content).unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "add review"]);
    }

    #[test]
    fn git_fixture_command_sets_temporary_home_environment_without_spawning() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("remote");
        std::fs::create_dir(&repository).unwrap();
        let command = git_command(&repository, &["rev-parse", "HEAD"]);
        let home = temp.path().join("git-test-home");
        let config = home.join(".config");

        for (key, expected) in [
            ("HOME", &home),
            ("USERPROFILE", &home),
            ("XDG_CONFIG_HOME", &config),
        ] {
            let actual = command
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(key))
                .and_then(|(_, value)| value);
            assert_eq!(
                actual,
                Some(expected.as_os_str()),
                "missing temporary {key}"
            );
            assert!(expected.starts_with(temp.path()));
            assert!(!expected.starts_with(&repository));
            assert!(expected.is_dir());
        }
    }

    fn git_command(path: &Path, args: &[&str]) -> Command {
        // Keep home configuration outside the repository's committed fixture contents.
        let home = path
            .parent()
            .expect("temporary fixture parent")
            .join("git-test-home");
        let config = home.join(".config");
        std::fs::create_dir_all(&config).unwrap();
        let mut command = Command::new("git");
        command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &config)
            .arg("-C")
            .arg(path)
            .args(args);
        command
    }

    fn git(path: &Path, args: &[&str]) {
        let output = git_command(path, args).output().unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(path: &Path, args: &[&str]) -> String {
        let output = git_command(path, args).output().unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}
