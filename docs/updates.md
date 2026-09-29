# Application updates

The native header follows T3 Code's explicit update flow. Stable release metadata
is checked two seconds after startup and every six hours. An available release
shows **Update available** without opening a dialog or fetching the installer.
Clicking downloads into a private update cache, with progress in the button.
After SHA-256 verification, **Restart to update** saves the current document and
ends any presentation. Failed saves keep the application open. Closing normally
does not install a downloaded update.

The separate `typsmthng-updater` executable receives an installation job over a
pipe. It verifies the artifact, acknowledges readiness, and waits for the app's
process to exit before changing the installation. It verifies the file again
before installing. The helper and job files live outside the installed app.

- AppImage: stage the new executable beside the current image, replace it, and
  relaunch. Replacement or process-spawn failure restores the previous image.
- macOS: mount the downloaded DMG read-only, verify the bundle's code signature,
  copy the entire app bundle, replace, and reopen it. Replacement or launch-command
  failure restores the previous bundle. The app must be installed outside the DMG
  in a location writable by the current user. Signature verification supports the
  existing ad-hoc release signing; it is not Developer ID authentication.
- Windows: copy the helper and its runtime DLLs out of the installation, wait for
  exit, run the per-user NSIS installer silently against the existing directory,
  then relaunch. NSIS owns replacement and its failure behavior.
- DEB, RPM and Flatpak: the update button explains that updates belong to the
  package manager and links to the release. It never overwrites a managed install.

Downloads have connection and overall timeouts, a 512 MiB limit, and must match
SHA256SUMS from the release. These checks provide integrity through the HTTPS
release channel, not independent publisher signature verification. Failed checks
never publish the partial artifact. Install errors are written beside the
artifact and shown when the app next opens. Cache directories older than seven
days are reclaimed on startup.

The updater first ships with the release containing this change. Older installed
versions need to install that release using their existing installer flow.

## Validation

`cargo test --locked --all-targets` covers checksums, failed downloads, helper exit
handoff, replacement, and rollback. The Linux helper integration tests use fake
executables in temporary directories, never the installed application.

The native interaction test checks that background discovery makes only one
metadata request, a button click downloads the artifact and checksum manifest,
and the installation stays untouched until restart:

```sh
dbus-run-session -- xvfb-run -a env GTK_A11Y=none GSK_RENDERER=cairo \
  cargo test --locked --bin typsmthng native_update_button_downloads_only_after_click \
  -- --ignored --test-threads=1
```

The release workflow runs the packaged helper on disposable Windows and macOS
installations. It checks that the helper waits for parent exit, replaces an old
binary, and relaunches the updated app. Windows also tests installation paths with
spaces and preservation of unrelated files. Publishing requires these checks.
