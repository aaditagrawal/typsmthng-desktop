# Fedora COPR packaging

COPR builds `typsmthng` from a source RPM on Fedora, using the system GTK runtime.
The source RPM contains the committed application source, the locked Rust crates,
their license notices, and the checksum-verified Typst 0.15.1 compiler. The binary
RPM builds without network access and installs its own compiler at
`/usr/lib/typsmthng/typst`, so it does not replace Fedora's `typst` package.

The shared `aaditagrawal/apps` project builds for active numbered Fedora x86_64
targets, including Fedora 43, 44 and 45 while COPR accepts builds. The RPM needs Rust 1.93.1 or newer, GTK 4.12 or newer,
GtkSourceView 5.4 or newer, and libadwaita 1.5 or newer.

## Build a source RPM

Run on Fedora with Rust, Cargo, `rpm-build`, Git, curl, tar, gzip, xz, and uv installed:

```sh
bash packaging/copr/build-srpm.sh "$PWD/build/copr"
mock -r fedora-44-x86_64 --rebuild build/copr/*.src.rpm
```

The builder archives `HEAD`, including its version and lockfile. Commit changes
before building; local edits and untracked files are excluded. It downloads
dependencies while preparing the source RPM. RPM `%build` uses `cargo --frozen`
and an empty Cargo home against vendored sources. CI additionally disables
networking while rebuilding the RPM and launches the installed Fedora package.

## Configure a COPR project

Create a Fedora account, sign in to [COPR](https://copr.fedorainfracloud.org/),
and obtain the configuration from [the API page](https://copr.fedorainfracloud.org/api/).
To submit builds, request **Builder** access on the [project permissions page](https://copr.fedorainfracloud.org/coprs/aaditagrawal/apps/permissions/) and wait for the owner to approve it.
The release workflow also needs permission to edit project settings when it enables a new Fedora target.

```sh
sudo dnf install copr-cli
copr-cli build aaditagrawal/apps build/copr/typsmthng-*.src.rpm
```

The project must exist before submission. The release workflow queries COPR for active numbered Fedora x86_64
targets, enables any missing targets and submits the source RPM for each one.

For GitHub Actions, configure:

| Repository setting | Value |
| --- | --- |
| Variable `COPR_PROJECT` | `aaditagrawal/apps` |
| Secret `COPR_CONFIG` | The complete INI configuration from COPR's API page |

`COPR_CONFIG` is written to a temporary file with mode 600 and removed after
submission. Keep it in repository secrets, rather than the source repository.

Once configured, the verified GitHub release workflow submits the same immutable
source commit to COPR after publication and waits for the COPR build result. If
`COPR_PROJECT` is unset, GitHub releases proceed without a COPR submission.
The job allows six hours for source preparation, the COPR queue, and compilation.
COPR submission also runs when the release reuses an existing verified candidate
and skips the package build. Failed, cancelled, and verification-only release runs
do not submit packages.

You can also run the **Fedora COPR** workflow manually with a source tag or commit.
Leave `submit` off to download a source RPM artifact for review, or enable it to
publish to the configured project. Use v0.1.7 or a later source with the COPR scripts.

COPR also supports SCM builds. Set the clone URL to this repository, the spec file
to `packaging/copr/typsmthng.spec.in`, and the SRPM build method to **make srpm**.
The `.copr/Makefile` prepares the complete source RPM. Pin the committish to a
release tag or full commit SHA for reproducible source selection.

## Install

After a successful COPR build:

```sh
sudo dnf copr enable aaditagrawal/apps
sudo dnf install typsmthng
```

If you previously enabled `aaditagrawal/typsmthng-desktop`, enable the shared project
once to receive new versions. Subsequent releases update through `dnf upgrade`. The native application does
not replace a package-managed installation through its download updater.
