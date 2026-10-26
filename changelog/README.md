# Changelog entries

Every pull request adds a short description of its user-visible changes. Entries
are grouped by the upcoming Cargo version and stay in that directory after
publication. They are bundled into the app for offline changelog display and
rendered into the GitHub release body by the release workflow.

Create an entry from the repository root:

```sh
uv run --no-project packaging/release/changelog.py add clearer-update-controls \
  --change "Check for updates from the home screen and editor."
```

Use a unique lowercase name and complete bullet lines. Describe what people can
now do or what stopped going wrong. Avoid commit titles, placeholders, and internal
implementation details. Keep entries concise. CI requires a newly added entry
for the current version on every PR and main push, so a previous entry cannot be
reused to satisfy the check. A version released before the change's base commit
must be bumped in Cargo.toml and Cargo.lock before adding new entries.

Preview the exact release body:

```sh
uv run --no-project packaging/release/changelog.py check
uv run --no-project packaging/release/changelog.py render
```

Historical notes in `releases/v*.md` remain supported. New releases use the
versioned entries here; no manual consolidation or deletion is required.
