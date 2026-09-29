//! Installation runs in a separate, dependency-light process after the app exits.
//! Keep this module independent of GTK: it is also compiled into the updater binary.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallJob {
    pub artifact: PathBuf,
    pub target: PathBuf,
    pub sha256: String,
}

fn failure(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut input = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = input.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn installed_target() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    return std::env::var_os("APPIMAGE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| failure("Use your package manager to update this installation."))
        .and_then(std::fs::canonicalize);
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe()?;
        let bundle = exe
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .filter(|path| path.extension().is_some_and(|ext| ext == "app"))
            .ok_or_else(|| failure("Run the installed application to update it."))?;
        if bundle.starts_with("/Volumes") {
            return Err(failure("Move typsmthng to Applications before updating."));
        }
        Ok(bundle.to_path_buf())
    }
    #[cfg(target_os = "windows")]
    {
        let exe = std::env::current_exe()?;
        let root = exe
            .parent()
            .and_then(Path::parent)
            .filter(|root| root.join("uninstall.exe").is_file())
            .ok_or_else(|| failure("Run the installed application to update it."))?;
        Ok(root.to_path_buf())
    }
}

/// Spawn the copied helper and arm it only after it acknowledges the job.
/// The intentionally retained pipe closes at process exit, so cancelling a save
/// must happen before calling this function.
pub fn handoff(job: &InstallJob) -> io::Result<()> {
    use std::io::BufRead;
    let directory = job
        .artifact
        .parent()
        .ok_or_else(|| failure("No update directory"))?;
    let helper_name = if cfg!(windows) {
        "typsmthng-updater.exe"
    } else {
        "typsmthng-updater"
    };
    let helper = std::env::current_exe()?.with_file_name(helper_name);
    let copied = directory.join(helper_name);
    std::fs::copy(&helper, &copied)?;
    #[cfg(windows)]
    for entry in std::fs::read_dir(
        helper
            .parent()
            .ok_or_else(|| failure("No runtime directory"))?,
    )? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        {
            std::fs::copy(entry.path(), directory.join(entry.file_name()))?;
        }
    }
    let log = std::fs::File::create(directory.join("install.log"))?;
    let mut command = Command::new(copied);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn()?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| failure("No updater input"))?;
    serde_json::to_writer(&mut input, job)?;
    input.write_all(b"\n")?;
    input.flush()?;
    let mut reply = String::new();
    io::BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| failure("No updater output"))?,
    )
    .read_line(&mut reply)?;
    if reply.trim() != "ready" {
        let _ = child.kill();
        return Err(failure(format!(
            "Updater did not start. See {}",
            directory.join("install.log").display()
        )));
    }
    input.write_all(b"install\n")?;
    input.flush()?;
    // Keep the OS handle alive until the parent process actually terminates.
    std::mem::forget(input);
    Ok(())
}

pub fn run_helper() -> io::Result<()> {
    use std::io::BufRead;
    let mut input = io::stdin().lock();
    let mut line = String::new();
    input.read_line(&mut line)?;
    let job: InstallJob = serde_json::from_str(&line)?;
    if hash_file(&job.artifact)? != job.sha256 {
        return Err(failure("Downloaded update changed before installation."));
    }
    println!("ready");
    io::stdout().flush()?;
    line.clear();
    input.read_line(&mut line)?;
    if line.trim() != "install" {
        return Err(failure("Update was not armed"));
    }
    io::copy(&mut input, &mut io::sink())?;
    // Recheck after waiting, before any replacement of the existing installation.
    let result = (|| {
        if hash_file(&job.artifact)? != job.sha256 {
            return Err(failure("Update checksum changed"));
        }
        install(&job)
    })();
    if let Err(error) = &result {
        if let Some(directory) = job.artifact.parent() {
            let _ = std::fs::write(directory.join("install-error.txt"), error.to_string());
        }
        // Replacement errors restore the old app before returning. Reopen it so
        // the persisted error can explain what happened and the user can retry.
        #[cfg(target_os = "linux")]
        {
            let _ = launch_appimage(&job.target);
        }
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("/usr/bin/open").arg(&job.target).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = Command::new(job.target.join("bin/typsmthng.exe")).spawn();
        }
    }
    result
}

/// Preserve the original until replacement and relaunch succeed. Staging beside
/// the target keeps rename atomic even when the download cache is on another disk.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn replace_and_launch(
    staged: &Path,
    target: &Path,
    launch: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let backup_dir = tempfile::Builder::new()
        .prefix(".typsmthng-backup-")
        .tempdir_in(
            target
                .parent()
                .ok_or_else(|| failure("No installation directory"))?,
        )?;
    let backup = backup_dir.path().join("previous");
    std::fs::rename(target, &backup)?;
    if let Err(error) = std::fs::rename(staged, target).and_then(|()| launch()) {
        if target.is_dir() {
            let _ = std::fs::remove_dir_all(target);
        } else {
            let _ = std::fs::remove_file(target);
        }
        if let Err(restore) = std::fs::rename(&backup, target) {
            let retained = backup_dir.keep();
            return Err(failure(format!(
                "{error}; could not restore: {restore}. Original retained in {}",
                retained.display()
            )));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install(job: &InstallJob) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let parent = job
        .target
        .parent()
        .ok_or_else(|| failure("No installation directory"))?;
    let stage = tempfile::NamedTempFile::new_in(parent)?;
    std::fs::copy(&job.artifact, stage.path())?;
    stage
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    stage.as_file().sync_all()?;
    let stage = stage.into_temp_path(); // Close the writable FD before exec (ETXTBSY).
    replace_and_launch(&stage, &job.target, || launch_appimage(&job.target))
}

#[cfg(target_os = "linux")]
fn launch_appimage(target: &Path) -> io::Result<()> {
    let mut command = Command::new(target);
    // AppImage's old mounted runtime must not contaminate the new process.
    for key in [
        "APPIMAGE",
        "APPDIR",
        "ARGV0",
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "GIO_MODULE_DIR",
        "GDK_PIXBUF_MODULE_FILE",
        "GSETTINGS_SCHEMA_DIR",
        "GTK_PATH",
        "GTK_EXE_PREFIX",
        "GTK_DATA_PREFIX",
        "XDG_DATA_DIRS",
        "GDK_PIXBUF_MODULEDIR",
        "GI_TYPELIB_PATH",
    ] {
        command.env_remove(key);
    }
    command.env("PATH", "/usr/local/bin:/usr/bin:/bin");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn install(job: &InstallJob) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    // NSIS requires /D last and unquoted, including paths containing spaces.
    // https://nsis.sourceforge.io/Docs/Chapter3.html#installerusage
    let mut destination = std::ffi::OsString::from("/D=");
    destination.push(&job.target);
    let status = Command::new(&job.artifact)
        .arg("/S")
        .raw_arg(destination)
        .status()?;
    if !status.success() {
        return Err(failure(format!("Installer failed: {status}")));
    }
    Command::new(job.target.join("bin/typsmthng.exe")).spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn install(job: &InstallJob) -> io::Result<()> {
    fn checked(program: &str, args: &[&std::ffi::OsStr]) -> io::Result<()> {
        let output = Command::new(program).args(args).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(failure(format!(
                "{program}: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
    let mount = tempfile::tempdir()?;
    checked(
        "/usr/bin/hdiutil",
        &[
            "attach".as_ref(),
            "-readonly".as_ref(),
            "-nobrowse".as_ref(),
            "-mountpoint".as_ref(),
            mount.path().as_os_str(),
            job.artifact.as_os_str(),
        ],
    )?;
    let result = (|| {
        let source = mount.path().join("typsmthng.app");
        checked(
            "/usr/bin/codesign",
            &[
                "--verify".as_ref(),
                "--deep".as_ref(),
                "--strict".as_ref(),
                source.as_os_str(),
            ],
        )?;
        let parent = job
            .target
            .parent()
            .ok_or_else(|| failure("No application directory"))?;
        let stage = tempfile::tempdir_in(parent)?;
        let staged = stage.path().join("typsmthng.app");
        checked("/usr/bin/ditto", &[source.as_os_str(), staged.as_os_str()])?;
        replace_and_launch(&staged, &job.target, || {
            checked("/usr/bin/open", &[job.target.as_os_str()])
        })
    })();
    let _ = checked(
        "/usr/bin/hdiutil",
        &["detach".as_ref(), mount.path().as_os_str()],
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hashes_complete_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        std::fs::write(&path, "abc").unwrap();
        assert_eq!(
            hash_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[cfg(unix)]
    #[test]
    fn restores_previous_installation_when_launch_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        let staged = dir.path().join("new");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&staged, "new").unwrap();
        assert!(replace_and_launch(&staged, &target, || Err(failure("launch failed"))).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "old");
    }
    #[cfg(unix)]
    #[test]
    fn replaces_and_launches_new_installation() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        let staged = dir.path().join("new");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&staged, "new").unwrap();
        replace_and_launch(&staged, &target, || {
            assert_eq!(std::fs::read_to_string(&target)?, "new");
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
