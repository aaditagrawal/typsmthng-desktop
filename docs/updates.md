# Application updates

The header always offers **Check for updates** on the home screen and in the
editor. The application menu also offers that action and **What's new**.
The native flow follows T3 Code's separate check, download, and restart actions.
Stable GitHub release metadata is checked two seconds after startup and every
six hours. Background checks fetch metadata only. **Update available** opens a
dialog with release notes; **Download update** starts the verified download.
Checks, progress, and retry controls remain available in the app. Downloads use
a private update cache, with progress in the dialog and header.
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
- DEB, RPM and Flatpak: **Open Software Updates** opens GNOME Software, KDE
  Discover, or the system AppStream handler. The dialog also shows the package
  manager command and links to the GitHub release. Repository updates can arrive
  after GitHub publication. Managed installs retain package-manager ownership.

Downloads have connection and overall timeouts, a 512 MiB limit, and must match
SHA256SUMS from the release. These checks provide integrity through the HTTPS
release channel, not independent publisher signature verification. Failed checks
never publish the partial artifact. Install errors are written beside the
artifact and shown when the app next opens. Cache directories older than seven
days are reclaimed on startup.

The updater handshake runs outside the GTK thread and times out if its helper
does not acknowledge readiness. Failed handoffs keep the verified download so
the restart can be retried.

## First launch after an update

The app bundles the changelog at build time, so it can display it without internet
access and after package-manager or manual upgrades. It compares the installed
version with `lastSeenVersion` in the user's app state. The changelog shows once
for a new installation or newer version. Skipped upgrades include all intervening
release notes. Dismissing the dialog records the acknowledgment; same-version
launches and downgrades do not repeat it. **What's new** reopens the current
version's notes at any time.

Each PR adds a new `changelog/VERSION/description.md` entry. Cargo fails if the
current version has no notes, CI checks entries against the PR base, and the
release workflow renders those same entries for GitHub publication. Historical
`releases/v*.md` notes remain supported. See [changelog entries](../changelog/README.md).

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

Linux CI also drives the actual GTK buttons and Enter key against a private fake
AppImage installation. It checks metadata-only discovery, verified download,
parent exit, file replacement, and relaunch through the real helper. It never
modifies an installed copy:

```sh
dbus-run-session -- xvfb-run -a env GTK_A11Y=none GSK_RENDERER=cairo \
  uv run --no-project packaging/test-in-app-update.py --app target/debug/typsmthng
```
