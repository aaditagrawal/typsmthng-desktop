# Fedora COPR packaging

COPR builds `typsmthng` from a source RPM on Fedora, using the system GTK runtime.
The source RPM contains the committed application source, the locked Rust crates,
their license notices, and the checksum-verified Typst 0.15.1 compiler. The binary
RPM builds without network access and installs its own compiler at
`/usr/lib/typsmthng/typst`, so it does not replace Fedora's `typst` package.

The first supported architecture is x86_64. Fedora 44 is the recommended build
chroot. Other chroots need Rust 1.93.1 or newer, GTK 4.12 or newer,
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
Follow [COPR's setup documentation](https://docs.pagure.org/copr.copr/user_documentation.html#quick-start).

```sh
sudo dnf install copr-cli
copr-cli create typsmthng --chroot fedora-44-x86_64
copr-cli build OWNER/typsmthng build/copr/typsmthng-*.src.rpm
```

The project must exist before submission. Select its desired x86_64 Fedora
chroots in COPR; the publisher uses that project configuration.

For GitHub Actions, configure:

| Repository setting | Value |
| --- | --- |
| Variable `COPR_PROJECT` | `OWNER/typsmthng` |
| Secret `COPR_CONFIG` | The complete INI configuration from COPR's API page |

`COPR_CONFIG` is written to a temporary file with mode 600 and removed after
submission. Keep it in repository secrets, rather than the source repository.

Once configured, the verified GitHub release workflow submits the same immutable
source commit to COPR after publication and waits for the COPR build result. If
`COPR_PROJECT` is unset, GitHub releases proceed without a COPR submission.

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
sudo dnf copr enable OWNER/typsmthng
sudo dnf install typsmthng
```

Subsequent releases update through `dnf upgrade`. The native application does
not replace a package-managed installation through its download updater.
