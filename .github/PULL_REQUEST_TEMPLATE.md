## Description

Brief description of changes.

## Checklist

Each item matches a CI job; the command is what to run locally.

- [ ] Tests pass (`cargo test --all-features` and `cargo test --no-default-features`)
- [ ] Clippy passes (`cargo clippy --all-targets --all-features -- -D warnings`)
- [ ] Formatting checked (`cargo fmt --all -- --check`)
- [ ] Docs build (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`)
- [ ] MSRV holds (`cargo +1.94 check --all-features`)
- [ ] Bare metal builds (`cargo build --no-default-features --target riscv32imc-unknown-none-elf`)
- [ ] Fuzz targets build and survive a short run (`cargo +nightly fuzz build`)
- [ ] Crate packages (`cargo package`)
- [ ] Dependency policy holds (`cargo deny check`)
- [ ] No unintended semver break against the last release (`cargo semver-checks`)
- [ ] Spec references cite the section; errata get a named case in `tests/conformance/`
- [ ] Documentation updated
- [ ] CHANGELOG.md updated

## Testing

Describe tests run and how to reproduce.
