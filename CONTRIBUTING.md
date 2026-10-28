# Contributing

## Checks

CI (`.github/workflows/ci.yml`) runs these on every pull request; run them
locally before opening one:

```sh
cargo test --all-features
cargo test --no-default-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo build --no-default-features --target riscv32imc-unknown-none-elf
cargo +nightly fuzz build
cargo deny check
cargo package
```

The MSRV is Rust 1.94 (`cargo +1.94 check --all-features`).

**The core stays `no_std` and allocation-free.** Anything that needs a heap
goes behind the `alloc` feature, and the `no-std` CI job builds the core and
`examples/no_std_responder` for bare-metal targets. Parsers never panic and
every new parser gets a fuzz target in `fuzz/`.

**Spec claims cite the section.** The MIDI-CI specifications are copyrighted
(MMA/AMEI); download them into `specs/` (git-ignored) and never commit them
or paste their text. Check byte layouts against the rendered tables, not a
text extraction, which drops rows. Contradictions in the specs go in the
errata log in `docs/SPEC.md`.

## Releasing

Releases are cut by pushing a version tag; `.github/workflows/release.yml`
does the rest.

1. On a branch, bump `version` in `Cargo.toml` and add a
   `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md`. That section becomes
   the GitHub release notes verbatim, so write it for users.
2. Merge the pull request into `main`.
3. Tag the merge commit on `main` and push the tag:

   ```sh
   git switch main && git pull
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

The workflow then:

1. **gate**: stops unless the tagged commit is on `main`, the tag equals
   `v` + the `Cargo.toml` version, and `CHANGELOG.md` has a dated, non-empty
   section for it.
2. **verify**, **msrv**, **semver** (in parallel): fmt, clippy, tests, the
   bare-metal build, docs and `cargo package`; the Rust 1.94 build; the API
   check against the latest release on crates.io (skipped for the first).
3. **publish**: in the `release` environment, publishes to crates.io using
   Trusted Publishing. A version already on crates.io is skipped, so
   re-running a tag is harmless.
4. **github-release**: creates the GitHub release from the tag, with the
   version's `CHANGELOG.md` section as notes and links to crates.io and docs.rs.

If a job fails, fix the cause on `main`, delete the tag (locally and with
`git push origin :refs/tags/vX.Y.Z`), and tag again. Nothing is published
before every check has passed.

### One-time setup

Publishing uses crates.io Trusted Publishing, so no registry token is stored
in the repository.

- **crates.io**: Trusted Publishing can only be added to a crate that exists,
  so the first version (0.0.1) is published by hand with `cargo publish`.
  Then, on the `midici` crate, Settings > Trusted Publishing > Add > GitHub,
  with owner `hxphsts`, repository `midici`, workflow `release.yml`,
  environment `release`.
- **GitHub**: repository Settings > Environments > New environment named
  `release`. Under deployment branches and tags, allow only tags matching
  `v*`. Optionally add required reviewers to approve each publish.
