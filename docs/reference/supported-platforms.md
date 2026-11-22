# Supported platforms: measurements

The release targets, the table and what is not supported are in the README's
[Supported platforms](../../README.md#supported-platforms); these are the measurements behind them.

- **The glibc floor, glibc 2.34**, is enforced: `scripts/check_glibc_floor.sh` fails the release
  smoke when a Linux binary requires a newer `GLIBC_` symbol version. Measured 2026-10-31: the
  v0.5.0 x86_64 binary requires at most `GLIBC_2.34`; it ran a review on AlmaLinux 9 (glibc
  2.34), Ubuntu 22.04 (2.35) and Debian 12 (2.36), and the loader refused it on Debian 11 (2.31).
  The arm64 binary, built on `ubuntu-22.04-arm`, passes the same check in CI (measured; built on
  `ubuntu-24.04-arm` it needed `GLIBC_2.39`, PR #79); it has not been run on an older arm64
  distribution (unmeasured). The 0.8.4 x86_64 binary, built on Fedora 44 (glibc 2.43) with the
  rustup toolchain's LLD, requires at most `GLIBC_2.34` (`readelf`, 2026-11-16; `GLIBC_2.39`
  only as the two weak symbols below); it has not been run on an older system.
- **On glibc 2.34 to 2.38 the loader prints one warning line to stderr per run**,
  ``weak version `GLIBC_2.39' not found``. The output and exit code are unchanged (measured on the
  three systems above). Rust's standard library references `pidfd_spawnp` and `pidfd_getpid`
  (glibc 2.39) as optional symbols; glibc 2.39 or newer prints nothing. Building from source on
  such a system also avoids it.
- **A source build is for the system that built it.** Linked with GNU ld on glibc 2.39 or newer
  (a distribution's own Rust package, for example), the binary requires `GLIBC_2.39` outright
  and does not start on an older glibc (measured 2026-11-10: built on Fedora 44, refused by the
  loader on Ubuntu 22.04). The release smoke holds every Linux release binary to the floor.
- **macOS minimums** are the ones the v0.5.0 binaries declare (`LC_BUILD_VERSION` minos 11.0 for
  arm64, `LC_VERSION_MIN_MACOSX` 10.12 for x86_64, read 2026-10-31). They are run only on the
  runner's current macOS, not on those versions.

## Runs on record

A target the README names as built and tested for a release has a receipt here: the
`release-receipt.json` that `scripts/release_local.sh build` writes (commit, toolchain, both
build hashes, every file's SHA-256), published with the release on GitLab and mirrored on
GitHub, after two equal builds, the release smoke (`--version` and a review document) and the
glibc check on the build machine; the whole local gate (`scripts/ci_local.sh`) passed on the
same commit first. Up to v0.8.3 the receipts are the `release.yml` runs of each GitHub release.
The test `every_platform_the_readme_names_has_a_run_on_record` (`cli/tests/crate_edges.rs`)
holds the README's table to this one.

| Release | Target | Commit, toolchain, binary | Receipt (`release-receipt.json`, SHA-256) |
| --- | --- | --- | --- |
| 0.9.0 | `x86_64-unknown-linux-gnu` | `7fb73f13`, Rust 1.99.1 in the Fedora 44 toolbox, `83158afe…` | `6a1b41acc6c28314aa7656a09bccfdcb27263e8d5502fa1bf158c82d6d7a4d16` |
| 0.8.4 | `x86_64-unknown-linux-gnu` | `76572ec0`, Rust 1.99.1 in the Fedora 44 toolbox, `0c93b3ed…` | `f086e473149977833820c4cb694f11861eadf78e42b3195441721e98d868f848` |

See also: [`RELEASE_AND_PUBLICATION`](../RELEASE_AND_PUBLICATION.md).
