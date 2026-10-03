//! Process-private font registration and local family discovery for GTK.
//!
//! Downloaded fonts are made visible to Pango (and therefore to GTK widgets
//! and GtkSourceView) without installing them system-wide:
//!
//! 1. Pango >= 1.56 exposes `pango_font_map_add_font_file`. It is looked up
//!    at runtime so older Pango (1.52 on Ubuntu 24.04) still links.
//! 2. Otherwise a platform fallback registers the file for this process:
//!    fontconfig application fonts on Linux/BSD, CoreText process scope on
//!    macOS, and `AddFontResourceExW(FR_PRIVATE)` on Windows. Pango's font
//!    map is then told to drop its caches.
//!
//! Everything here touches the thread-default Pango font map and therefore
//! must run on the GTK main thread after GTK is initialized.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gtk::pango;
use gtk::prelude::*;

use crate::backend::error::{BackendError, Result};

thread_local! {
    static REGISTERED: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new());
}

/// The font map GTK widgets draw with (the thread-default PangoCairo map).
pub fn default_font_map() -> Result<pango::FontMap> {
    if !gtk::is_initialized_main_thread() {
        return Err(BackendError::FontRegistration(
            "GTK must be initialized and fonts registered on the main thread".into(),
        ));
    }
    gtk::Label::new(None)
        .pango_context()
        .font_map()
        .ok_or_else(|| BackendError::FontRegistration("GTK has no Pango font map".into()))
}

/// Register font files for this process only. Idempotent: files that were
/// already registered are skipped. Every file is attempted; the first error
/// is returned after the rest are processed.
pub fn register_app_font_files(paths: &[PathBuf]) -> Result<()> {
    let font_map = default_font_map()?;
    let pending = REGISTERED.with_borrow(|registered| {
        let mut seen = HashSet::new();
        paths
            .iter()
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
            .filter(|path| !registered.contains(path) && seen.insert(path.clone()))
            .collect::<Vec<_>>()
    });
    let mut first_error = None;
    let mut needs_refresh = false;
    for path in pending {
        let outcome = if !path.is_file() {
            Err(format!("{} is not a font file", path.display()))
        } else {
            match platform::pango_add_font_file(&font_map, &path) {
                Some(Ok(())) => Ok(()),
                pango_result => platform::register_font_file(&path)
                    .map(|()| needs_refresh = true)
                    .map_err(|error| match pango_result {
                        Some(Err(pango_error)) => format!("{pango_error}; {error}"),
                        _ => error,
                    }),
            }
        };
        match outcome {
            Ok(()) => REGISTERED.with_borrow_mut(|registered| {
                registered.insert(path);
            }),
            Err(error) => {
                first_error.get_or_insert(BackendError::FontRegistration(error));
            }
        }
    }
    if needs_refresh {
        platform::refresh_font_map(&font_map);
    }
    first_error.map_or(Ok(()), Err)
}

/// Whether a font file has been registered by [`register_app_font_files`].
pub fn is_registered(path: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    REGISTERED.with_borrow(|registered| registered.contains(&path))
}

/// Installed (and app-registered) font families known to Pango, sorted
/// case-insensitively without duplicates.
pub fn list_local_families(monospace_only: bool) -> Vec<String> {
    let Ok(font_map) = default_font_map() else {
        return Vec::new();
    };
    let mut families = font_map
        .list_families()
        .into_iter()
        .filter(|family| !monospace_only || family.is_monospace())
        .map(|family| family.name().to_string())
        .filter(|name| !name.trim().is_empty())
        .collect::<Vec<_>>();
    families.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    families
}

/// Whether Pango can resolve `family` exactly (not via fallback).
pub fn family_is_available(family: &str) -> bool {
    default_font_map().is_ok_and(|font_map| {
        font_map
            .list_families()
            .iter()
            .any(|candidate| candidate.name().eq_ignore_ascii_case(family))
    })
}

#[cfg(unix)]
mod symbols {
    use std::ffi::{c_void, CStr};

    pub fn lookup(_library: &str, name: &CStr) -> Option<*mut c_void> {
        // SAFETY: dlsym with RTLD_DEFAULT only reads the global symbol table.
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        (!symbol.is_null()).then_some(symbol)
    }
}

#[cfg(windows)]
mod symbols {
    use std::ffi::{c_char, c_void, CStr};

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `library` is the MSYS2 DLL name, e.g. `libpango-1.0-0.dll`.
    pub fn lookup(library: &str, name: &CStr) -> Option<*mut c_void> {
        let wide = library
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: both calls only inspect already-loaded modules.
        unsafe {
            let module = GetModuleHandleW(wide.as_ptr());
            if module.is_null() {
                return None;
            }
            let symbol = GetProcAddress(module, name.as_ptr());
            (!symbol.is_null()).then_some(symbol)
        }
    }
}

mod platform {
    use std::ffi::{c_char, c_void, CString};
    use std::path::Path;

    use glib::translate::{from_glib_full, ToGlibPtr};
    use gtk::pango;
    use gtk::prelude::*;

    use super::symbols;

    type PangoFontMapPtr = *mut pango::ffi::PangoFontMap;

    fn path_to_cstring(path: &Path) -> Result<CString, String> {
        #[cfg(unix)]
        let bytes = {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes().to_vec()
        };
        // GLib filenames are UTF-8 on Windows.
        #[cfg(not(unix))]
        let bytes = path
            .to_str()
            .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?
            .as_bytes()
            .to_vec();
        CString::new(bytes).map_err(|_| format!("{} contains a NUL byte", path.display()))
    }

    /// `None` when this Pango lacks `pango_font_map_add_font_file` (< 1.56).
    pub fn pango_add_font_file(
        font_map: &pango::FontMap,
        path: &Path,
    ) -> Option<Result<(), String>> {
        type AddFontFile = unsafe extern "C" fn(
            PangoFontMapPtr,
            *const c_char,
            *mut *mut glib::ffi::GError,
        ) -> glib::ffi::gboolean;
        let symbol = symbols::lookup("libpango-1.0-0.dll", c"pango_font_map_add_font_file")?;
        let filename = match path_to_cstring(path) {
            Ok(filename) => filename,
            Err(error) => return Some(Err(error)),
        };
        // SAFETY: the symbol has the documented Pango 1.56 signature; the
        // font map and filename outlive the call.
        let add = unsafe { std::mem::transmute::<*mut c_void, AddFontFile>(symbol) };
        let mut error = std::ptr::null_mut();
        let added = unsafe { add(font_map.to_glib_none().0, filename.as_ptr(), &mut error) };
        if added != glib::ffi::GFALSE {
            return Some(Ok(()));
        }
        let message = if error.is_null() {
            format!("Pango could not add {}", path.display())
        } else {
            // SAFETY: Pango transferred ownership of the GError to us.
            unsafe { from_glib_full::<_, glib::Error>(error) }.to_string()
        };
        Some(Err(message))
    }

    fn is_fontconfig_map(font_map: &pango::FontMap) -> bool {
        glib::Type::from_name("PangoFcFontMap").is_some_and(|fc| font_map.type_().is_a(fc))
    }

    /// Drop Pango's cached font lists so newly registered files are found.
    pub fn refresh_font_map(font_map: &pango::FontMap) {
        type ConfigChanged = unsafe extern "C" fn(PangoFontMapPtr);
        if is_fontconfig_map(font_map) {
            if let Some(symbol) =
                symbols::lookup("libpangoft2-1.0-0.dll", c"pango_fc_font_map_config_changed")
            {
                // SAFETY: documented PangoFcFontMap API; the map is a PangoFcFontMap.
                unsafe {
                    let changed = std::mem::transmute::<*mut c_void, ConfigChanged>(symbol);
                    changed(font_map.to_glib_none().0);
                }
                return;
            }
            font_map.changed();
            return;
        }
        font_map.changed();
        // CoreText and DirectWrite maps enumerate fonts once. Resetting the
        // default lets widgets created afterwards see the new families.
        type SetDefault = unsafe extern "C" fn(PangoFontMapPtr);
        if let Some(symbol) = symbols::lookup(
            "libpangocairo-1.0-0.dll",
            c"pango_cairo_font_map_set_default",
        ) {
            // SAFETY: NULL resets the thread default to a fresh font map.
            unsafe {
                let reset = std::mem::transmute::<*mut c_void, SetDefault>(symbol);
                reset(std::ptr::null_mut());
            }
        }
    }

    /// fontconfig application fonts; Pango's fontconfig map reads them.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn register_font_file(path: &Path) -> Result<(), String> {
        type AppFontAddFile = unsafe extern "C" fn(*mut c_void, *const u8) -> std::ffi::c_int;
        let symbol = symbols::lookup("", c"FcConfigAppFontAddFile")
            .ok_or("fontconfig is not loaded in this process")?;
        let filename = path_to_cstring(path)?;
        // SAFETY: documented fontconfig API; a NULL config selects the
        // current configuration, which Pango's default font map uses.
        let added = unsafe {
            let add = std::mem::transmute::<*mut c_void, AppFontAddFile>(symbol);
            add(std::ptr::null_mut(), filename.as_ptr().cast())
        };
        if added == 0 {
            return Err(format!("fontconfig rejected {}", path.display()));
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn register_font_file(path: &Path) -> Result<(), String> {
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            fn CFURLCreateFromFileSystemRepresentation(
                allocator: *const c_void,
                buffer: *const u8,
                length: isize,
                is_directory: u8,
            ) -> *const c_void;
            fn CFErrorGetCode(error: *const c_void) -> isize;
            fn CFRelease(object: *const c_void);
        }
        #[link(name = "CoreText", kind = "framework")]
        extern "C" {
            fn CTFontManagerRegisterFontsForURL(
                url: *const c_void,
                scope: u32,
                error: *mut *const c_void,
            ) -> u8;
        }
        const PROCESS_SCOPE: u32 = 1;
        const ALREADY_REGISTERED: isize = 105;
        let filename = path_to_cstring(path)?;
        let bytes = filename.as_bytes();
        // SAFETY: CoreFoundation create/release pairs; `error` is only read
        // when CoreText reports failure and is released afterwards.
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                0,
            );
            if url.is_null() {
                return Err(format!("invalid font path {}", path.display()));
            }
            let mut error = std::ptr::null();
            let registered = CTFontManagerRegisterFontsForURL(url, PROCESS_SCOPE, &mut error);
            CFRelease(url);
            if registered != 0 {
                return Ok(());
            }
            let code = if error.is_null() {
                0
            } else {
                let code = CFErrorGetCode(error);
                CFRelease(error);
                code
            };
            if code == ALREADY_REGISTERED {
                Ok(())
            } else {
                Err(format!(
                    "CoreText could not register {} (error {code})",
                    path.display()
                ))
            }
        }
    }

    #[cfg(windows)]
    pub fn register_font_file(path: &Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "gdi32")]
        extern "system" {
            fn AddFontResourceExW(
                name: *const u16,
                flags: u32,
                reserved: *mut c_void,
            ) -> std::ffi::c_int;
        }
        const FR_PRIVATE: u32 = 0x10;
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: NUL-terminated wide path; private fonts vanish at exit.
        let added = unsafe { AddFontResourceExW(wide.as_ptr(), FR_PRIVATE, std::ptr::null_mut()) };
        if added == 0 {
            return Err(format!("Windows could not load {}", path.display()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fonts::GoogleFontCache;

    fn test_font(name: &str) -> Option<PathBuf> {
        let directory = std::env::var_os("TYPSMTHNG_TEST_FONT_DIR")?;
        let path = PathBuf::from(directory).join(name);
        path.is_file().then_some(path)
    }

    /// GTK can only be initialized on one thread per process, so all display
    /// checks live in one test. Run with a display, e.g.
    /// `TYPSMTHNG_TEST_FONT_DIR=... xvfb-run cargo test --lib app_fonts -- --ignored`
    /// where the directory contains `Fraunces-Regular.ttf` (not installed).
    #[test]
    #[ignore = "needs a display and TYPSMTHNG_TEST_FONT_DIR"]
    fn registered_font_is_resolved_by_pango() {
        gtk::init().expect("GTK display");
        let all = list_local_families(false);
        let mono = list_local_families(true);
        assert!(!all.is_empty());
        assert!(mono.len() < all.len());
        assert!(mono.iter().all(|name| all.contains(name)));
        let mut sorted = all.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(all, sorted);

        let font = test_font("Fraunces-Regular.ttf").expect("Fraunces-Regular.ttf test font");
        assert!(
            !family_is_available("Fraunces"),
            "test font must not be installed system-wide"
        );
        register_app_font_files(&[font.clone(), font.clone()]).unwrap();
        assert!(is_registered(&font));
        assert!(family_is_available("Fraunces"));
        assert!(list_local_families(false)
            .iter()
            .any(|name| name == "Fraunces"));
        assert!(!list_local_families(true)
            .iter()
            .any(|name| name == "Fraunces"));

        let context = gtk::Label::new(None).pango_context();
        let description = pango::FontDescription::from_string("Fraunces 14");
        let font = context
            .load_font(&description)
            .expect("Pango loads a font for Fraunces");
        assert_eq!(font.describe().family().as_deref(), Some("Fraunces"));

        // A second call is a no-op and a missing file reports an error.
        register_app_font_files(&[PathBuf::from("/nonexistent/font.ttf")]).unwrap_err();

        // End to end: download UI styles of a Google family and register them.
        if std::env::var_os("TYPSMTHNG_TEST_NETWORK").is_some() {
            let directory = tempfile::tempdir().unwrap();
            let cache = GoogleFontCache::with_directory(directory.path());
            assert!(!family_is_available("Bricolage Grotesque"));
            let files = cache.ensure_family_files("Bricolage Grotesque").unwrap();
            assert!(!files.is_empty() && files.len() <= 4, "{files:?}");
            register_app_font_files(&files).unwrap();
            assert!(family_is_available("Bricolage Grotesque"));
            assert_eq!(
                cache.cached_family_files("Bricolage Grotesque"),
                Some(files)
            );
            let italics = cache.ensure_family_files("Fraunces").unwrap();
            assert_eq!(italics.len(), 4, "{italics:?}");
            cache
                .ensure_family_files("Not A Real Family 123")
                .unwrap_err();
        }
    }
}
