# Release process

Status: **design**, written 2026-04-26 against truce 0.13.1, updated
2026-04-27 against truce 0.14.1 with crates.io publishing for
`cargo-truce` and pre-1.0 `preview/{major}.{minor}` train branches.

How to cut a `truce` release that scaffolded plugins can pin to. Every
release is **both** a Git tag (`v{major}.{minor}.{patch}`) and a
release branch (`preview/{major}.{minor}`):

- The **tag** is the immutable snapshot. CI artifacts, the
  `cargo install --tag v0.14.2 cargo-truce` recipe, and any
  `tag = "v0.14.2"` pin in a user's `Cargo.toml` resolve to it.
- The **branch** is what users pin via `branch =
  "preview/0.14"` to float patches automatically. Each `0.14.x`
  patch release fast-forwards `preview/0.14` to the new tag.

The branch costs one extra `git push` per release and gives users
the cargo-flavoured upgrade path they expect from semver.

---

## When to release

- **Patch (`0.14.1` → `0.14.2`):** bug fixes, doc changes, and any
  *additive* API change that doesn't break existing scaffolded
  plugins. Goes onto the existing `preview/0.14` branch.
- **Minor (`0.14.x` → `0.15.0`):** new features that change the
  surface in a forwards-compatible way for new code, or fix a
  bug in a way that requires a recompile (e.g. ABI change in a
  format wrapper). Cuts a new `preview/0.15` branch; the old
  `preview/0.14` branch stays alive for users who haven't migrated.
- **Major (`0.x` → `1.0`):** deferred until the surface settles.
  See [`beta-checklist.md`](beta-checklist.md).

Today truce is on the `0.14.x` train. We commit to keeping the
**current `preview/{major}.{minor}` branch open for one minor
release after the next** — i.e. when `0.15.0` lands, `preview/0.14`
keeps receiving compat patches until `0.16.0` cuts, then sunsets.

---

## Cutting a release

Pre-flight from a clean working tree on `main`:

```sh
# 1. Verify clean state
git checkout main
git pull --ff-only
git status                                   # working tree must be clean
cargo test --workspace                       # full test suite green
cargo clippy --workspace -- -D warnings      # no clippy warnings
```

### Patch release (most common)

You want `0.14.1` → `0.14.2`, branch `preview/0.14` already exists.

```sh
# 1. Bump the version in the workspace root + cargo-truce
#    (cargo-truce has its own [package].version that needs bumping
#    in lock-step with the workspace.)
sed -i '' 's/"0.14.1"/"0.14.2"/g' Cargo.toml crates/cargo-truce/Cargo.toml

# 2. Update CHANGELOG and the `Status: ... against truce 0.14.x`
#    headers in design docs that pin to the version
$EDITOR CHANGELOG.md
$EDITOR ../truce-docs/docs/internal/install-scope.md  # one-line version stamp

# 3. Refresh Cargo.lock with the bumped versions
cargo check --workspace

# 4. Commit on main
git add -A
git commit -m "Release v0.14.2"

# 5. Tag and fast-forward the release branch
git tag -a v0.14.2 -m "truce 0.14.2"
git checkout preview/0.14
git merge --ff-only main                     # branch was at v0.14.1; FF to main
git checkout main

# 6. Publish to crates.io (see the section below — must happen before
#    the push so a failed publish doesn't leave a tag without a
#    matching crates.io artifact)
cargo publish -p truce-shim-types
sleep 30                                     # crates.io index lag; see notes below
cargo publish -p cargo-truce

# 7. Push branch, release branch, and tag in one go
git push origin main preview/0.14 v0.14.2
```

If `git merge --ff-only` rejects (release branch has commits not on
main), you've drifted — see [Hotfixes](#hotfixes) below.

### Minor release (new release branch)

You want `0.14.x` → `0.15.0`. Same first three steps, then:

```sh
# 4. Commit on main
git add -A
git commit -m "Release v0.15.0"

# 5. Cut the new release branch and tag
git tag -a v0.15.0 -m "truce 0.15.0"
git branch preview/0.15 v0.15.0              # new branch from the tag

# 6. Publish to crates.io
cargo publish -p truce-shim-types
sleep 30
cargo publish -p cargo-truce

# 7. Push everything
git push origin main preview/0.15 v0.15.0

# 8. Mark the previous train as sunset-pending in CHANGELOG. The
#    branch keeps receiving `0.14.x` patches for one minor cycle
#    (i.e. until 0.16.0 cuts).
```

Don't delete `preview/0.14` — users still have `branch = "preview/0.14"`
in their `Cargo.toml`. Sunset by stopping new patches, not by removing
the ref.

---

## Hotfixes

The release branch and `main` can diverge when a security or
correctness fix needs to ship before the next normal release on
`main` is ready. Workflow:

```sh
# 1. Branch off the existing release line for the fix
git checkout preview/0.14
git checkout -b hotfix/0.14.3-loader-crash

# 2. Apply the minimal fix; resist scope creep — anything beyond the
#    bug should land on main and wait for the next minor.
$EDITOR crates/truce-loader/...
git commit -am "Fix: loader crash on AAX session reload (#1234)"

# 3. Bump to 0.14.3 on the hotfix branch only
sed -i '' 's/"0.14.2"/"0.14.3"/g' Cargo.toml crates/cargo-truce/Cargo.toml
cargo check --workspace
git commit -am "Release v0.14.3"

# 4. Merge hotfix into the release branch and tag from there
git checkout preview/0.14
git merge --no-ff hotfix/0.14.3-loader-crash
git tag -a v0.14.3 -m "truce 0.14.3 (hotfix)"

# 5. Backport to main. Cherry-pick the fix commit (not the version
#    bump — main is on whatever 0.15.0-dev version it's tracking).
git checkout main
git cherry-pick <fix-commit-sha>             # not the version bump

# 6. Publish from the release branch (only re-publish crates whose
#    bytes actually changed; truce-shim-types rarely does in a hotfix)
git checkout preview/0.14
cargo publish -p truce-shim-types --dry-run  # confirm whether re-publish is needed
cargo publish -p cargo-truce
git checkout main

# 7. Push everything
git push origin preview/0.14 v0.14.3 main
git branch -d hotfix/0.14.3-loader-crash
```

The cherry-pick keeps `main` and the release branch in sync on the
fix while letting their version numbers drift.

---

## What scaffolded plugins resolve to

After the `git push`, a user's `Cargo.toml` resolves as:

| Pin form (in user's `Cargo.toml`) | Resolves to |
|-----------------------------------|-------------|
| `git = "https://github.com/truce-audio/truce"` | latest commit on `main` (no pin — every `cargo update` moves) |
| `git = "...", tag = "v0.14.2"` | exact tag, immutable |
| `git = "...", rev = "<sha>"` | exact commit, immutable |
| `git = "...", branch = "preview/0.14"` | latest patch in the `0.14.x` train (auto-tracks `0.14.3`, `0.14.4`, …; stops at `0.15`) |

`cargo truce new` emits the **train branch** form:

```toml
truce = { git = "https://github.com/truce-audio/truce", branch = "preview/0.14" }
```

This auto-tracks patch releases on the train the user scaffolded
against and stops at the next minor — the lowest-friction upgrade
path that's still bounded by semver. Users who want bit-for-bit
reproducibility can pin to a tag manually after scaffolding.

The branch name is hard-coded in `crates/cargo-truce/src/scaffold.rs`
today. When cutting a new minor (`preview/0.15`, `preview/0.16`, …)
the scaffold templates need a parallel bump — they don't read
`[workspace.package].version` yet.

> **TODO: derive the scaffold pin from `[workspace.package].version`.**
> Right now the `branch = "preview/0.14"` string in
> `crates/cargo-truce/src/scaffold.rs` is hard-coded. When 0.15 cuts,
> the scaffold templates need a manual `s/preview\/0.14/preview\/0.15/`
> pass — easy to forget, and a forgotten bump means new scaffolds
> silently track the *previous* train.
>
> Fix: read `env!("CARGO_PKG_VERSION")` at compile time (cargo-truce's
> own version, which the release process keeps in lock-step with the
> workspace), parse the major.minor, format `preview/{major}.{minor}`,
> interpolate into the templates. Single source of truth; impossible
> to forget. Track in this doc when implemented.
>
> Follow-up to evaluate once that lands: emit a commented-out
> `# tag = "vX.Y.Z"` line above the branch dep so users who want
> reproducibility can flip the comment without learning the cargo-by-tag
> dance from scratch. Self-documenting; doesn't change default
> floating-on-the-train behavior.

The README's `cargo install` recipe currently emits the bare git
form too:

```sh
cargo install --git https://github.com/truce-audio/truce cargo-truce
```

Once cargo-truce is on crates.io (see [Crates.io publishing](#cratesio-publishing)
above), the supported recipe becomes `cargo install cargo-truce`.
The branch / tag forms below remain available for users who want
to install from a specific release line or commit:

```sh
cargo install cargo-truce                                     # crates.io (preferred)
cargo install --git https://github.com/truce-audio/truce \
              --branch preview/0.14 cargo-truce               # git, latest 0.14.x
cargo install --git https://github.com/truce-audio/truce \
              --tag v0.14.2 cargo-truce                       # git, exact pin
```

Updating the README to lead with the crates.io form is part of the
publish landing — covered by the checklist's "GitHub release notes
drafted from CHANGELOG" item.

---

## Tag hygiene

- **Annotated tags only** (`git tag -a`), never lightweight. Annotated
  tags carry a tagger identity, date, and message; they show up in
  `git describe` and GitHub's release UI; they survive
  `git push --tags`. Lightweight tags don't.
- **Never force-move a tag.** Once `v0.14.2` is pushed it's
  immutable. If the release is broken, cut `v0.14.3` with the fix —
  forcing a tag breaks every user who already pinned to it.
- **Sign tags** (`git tag -s`) once we have a release-signing key set
  up. Not blocking for 0.x; a phase-2 hardening task tracked in
  [`hardening.md`](hardening.md).

---

## Branch hygiene

- **Fast-forward only.** The release branch is a moving pointer at
  the *latest patch tag* on its train. `git merge --ff-only` is the
  invariant; a merge that wouldn't fast-forward indicates drift
  (handled by hotfix workflow above), not a code-review situation
  to resolve on the branch.
- **Never delete a release branch.** Once a user pins to it, the
  ref is part of our public API. Sunsetting means stopping pushes,
  not removing the branch.
- **Don't squash-merge into release branches.** Always preserve the
  exact tagged commit so `git log preview/0.14 --first-parent`
  reads as a clean list of releases.

---

## Crates.io publishing

`cargo-truce` is the one crate users `cargo install`, so it lives on
crates.io. The framework crates (`truce`, `truce-gui`, format
wrappers, etc.) stay git-only — they transitively depend on `baseview`
which is git-only — and scaffolded plugins consume them via the git
ref + release-branch pin documented above. See
[`publishing-crates.md`](publishing-crates.md) for the full
git-vs-crates-io split.

### What gets published

| Crate | Why |
|-------|-----|
| `truce-shim-types` | Direct dep of `cargo-truce`; cargo strips the `path =` half of the workspace dep on publish, so the version must already be on crates.io. |
| `cargo-truce` | The `cargo install cargo-truce` target. |

If a future release adds a new dep to `cargo-truce` that lives in this
repo, it joins the publish list (and must publish before `cargo-truce`).

### One-time setup

- Run `cargo login <token>` once per release machine. Token comes from
  https://crates.io/me — scope it to "publish-update" plus "publish-new"
  for first-time publish of `truce-shim-types`.
- The crates.io account must own both crate names. First publish
  claims them.
- `truce-shim-types/Cargo.toml` needs `repository.workspace = true`,
  `homepage.workspace = true`, `categories.workspace = true`, and
  `keywords` set. crates.io rejects the upload without
  license + description + repository. (Already inherits
  `license.workspace = true`; verify before the first publish.)

### Publish recipe

Runs from the version-bumped commit. The tag should already exist
locally so a publish failure is recoverable (delete the local tag,
fix, retry — nothing has been pushed yet).

```sh
# Sanity check what each upload will contain. --dry-run is a full
# package + verify pass; catches missing metadata, .gitignore'd
# files, dirty trees, version conflicts.
cargo publish -p truce-shim-types --dry-run
cargo publish -p cargo-truce --dry-run

# Real publish, in dependency order. crates.io's index has up to
# ~30s of CDN lag between accepting a publish and making the new
# version visible to a downstream resolver, so insert a sleep
# between the two — otherwise `cargo publish -p cargo-truce` can
# fail to find the just-published `truce-shim-types`.
cargo publish -p truce-shim-types
sleep 30
cargo publish -p cargo-truce
```

### Failure modes

- **`error: failed to verify package`** during `cargo publish` —
  almost always a `Cargo.toml` metadata gap (missing `description`,
  `license`, or `repository`). Fix on `main`, amend the release
  commit, retry. The local tag has not been pushed; either re-tag
  (annotated tags are not yet immutable on the remote) or move
  the tag to the amended commit with `git tag -fa vX.Y.Z`.
- **`error: api errors: crate version X.Y.Z is already uploaded`** —
  someone already published this version. Bump to the next patch
  and start again. crates.io versions are immutable; you cannot
  re-publish over them.
- **`error: failed to select a version for ...`** during the
  `cargo-truce` publish — the index hasn't propagated
  `truce-shim-types` yet. Wait 30–60s and retry. The first publish
  is idempotent for the consumer.
- **Yanking.** If a published `cargo-truce` turns out to be broken,
  `cargo yank --version X.Y.Z -p cargo-truce` hides it from new
  installs without removing it (existing `Cargo.lock` files keep
  resolving). Fix forward with the next patch — never re-publish
  the same version.

### What stays git-only

Every other framework crate. The branch pin (`branch =
"preview/0.14"`) emitted by `cargo truce new` is the supported
distribution channel for the framework. Tags and release branches
remain authoritative for any plugin that wants to track a fork or
an unreleased commit; nothing about crates.io publishing changes
that contract.

---

## Checklist

Pin this on the wall before any release:

- [ ] Working tree clean on `main`
- [ ] `cargo test --workspace` green
- [ ] `cargo clippy --workspace -- -D warnings` clean
- [ ] All three platform CI runs green on the release commit
- [ ] CHANGELOG entry written
- [ ] Workspace + `cargo-truce` versions bumped in lock-step
- [ ] `Cargo.lock` regenerated (`cargo check --workspace`)
- [ ] Annotated tag created (`git tag -a vX.Y.Z`)
- [ ] Release branch fast-forwarded to the tag
- [ ] `cargo publish -p truce-shim-types --dry-run` clean
- [ ] `cargo publish -p cargo-truce --dry-run` clean
- [ ] `truce-shim-types` published to crates.io
- [ ] `cargo-truce` published to crates.io (after the 30s index wait)
- [ ] `main`, `release/X.Y`, and `vX.Y.Z` all pushed in one
      `git push` (atomic from the user's perspective — they never
      see a tag without its branch update)
- [ ] GitHub release notes drafted from CHANGELOG
- [ ] `cargo install cargo-truce` smoke-tested from a clean machine
      (or `cargo install --force cargo-truce` locally) so the
      crates.io artifact is verified end-to-end

---

## See also

- [`publishing-crates.md`](publishing-crates.md) — what's on
  crates.io vs git-only
- [`beta-checklist.md`](beta-checklist.md) — outstanding work
  before `1.0`
- [`install-scope.md`](install-scope.md) — version-stamp convention
  used by design docs
