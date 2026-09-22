# qemu-wrapper

Wrap QEMU and convert a SIGINT (Ctrl+C) into a `system_powerdown`
[monitor command](https://www.qemu.org/docs/master/system/monitor.html#commands).

Without a preceeding `system_powerdown`, QEMU will flush its disks and halt the machine without
notifying the operating system.

## Building

```sh
nix build
```

## Testing

Includes lints, formatting, and unit and integration tests:
```sh
nix flake check
```

## Updating dependencies

Commit each logically-distinct change separately and ensure `nix flake check` passes after each.

### Crates

Apply all semver-compatible updates in both workspaces:
```sh
nix develop
(cd test-guest && cargo update --verbose)
cargo update --verbose
```
The `--verbose` output ends with a list of direct dependencies that were left `Unchanged` because
a newer, semver-incompatible version is `available`.

For each crate listed as `Unchanged`, bump `version` in the relevant `Cargo.toml` to the available
version, check https://lib.rs/crates/<CRATE>/versions for any new feature flags adding any that are
needed, and re-run the `cargo update` commands.

### Flake inputs

Update the inputs of both flakes, `test-guest` first so that the root flake's `follows` and the
`test-guest` lock agree:
```sh
(cd test-guest && nix flake update)
nix flake update
```
