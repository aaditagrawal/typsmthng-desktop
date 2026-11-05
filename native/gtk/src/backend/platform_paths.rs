//! The directory conventions used by application state and font caches.

use std::path::PathBuf;

#[cfg(target_os = "linux")]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(passwd_home)
}

#[cfg(target_os = "linux")]
fn passwd_home() -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    // Ask NSS for the current uid, independently of login-name environment variables.
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let size = usize::try_from(suggested).unwrap_or(512);
    let mut record = MaybeUninit::<libc::passwd>::uninit();
    // NSS initializes the bytes referenced by the returned passwd record.
    let mut buffer = Box::<[u8]>::new_uninit_slice(size);
    let mut found = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            record.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return None;
    }
    // A successful getpwuid_r initializes record and its pointers into
    // buffer. Copy the NUL-terminated path before dropping that buffer.
    let directory = unsafe { record.assume_init() }.pw_dir;
    if directory.is_null() {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(directory) }.to_bytes();
    (!bytes.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(bytes)))
}

#[cfg(target_os = "linux")]
fn cache_dir() -> Option<PathBuf> {
    let home = home_dir()?;
    Some(
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".cache")),
    )
}

/// App data root. Preserve existing XDG_DATA_HOME compatibility, including
/// relative overrides, because changing it would relocate persisted settings.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    let home = home_dir()?;
    #[cfg(not(target_os = "linux"))]
    let base = directories::BaseDirs::new()?;
    #[cfg(not(target_os = "linux"))]
    let home = base.home_dir();
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
    })
}

pub fn google_font_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        cache_dir().map(|cache| cache.join("dev.typsmthng.desktop/google-fonts"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        directories::BaseDirs::new()
            .map(|base| base.cache_dir().join("dev.typsmthng.desktop/google-fonts"))
    }
}

pub fn update_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        cache_dir().map(|cache| cache.join("typsmthng/updates"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        directories::ProjectDirs::from("dev", "typsmthng", "typsmthng")
            .map(|project| project.cache_dir().join("updates"))
    }
}
