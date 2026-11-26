# bezel

Bezel is a lightweight RDP client written in Rust using [Rune](https://github.com/anomalyco/rune).

## Read first

1. `CONTEXT.md`: the domain terms (Connection, New host, Session, Tab, Main window, Session window, Group, Credential, Trusted certificate). Use them in code, copy and commits.
2. `docs/adr/`: recorded decisions.
3. `docs/spec.md`: the v1 product and architecture contract, with milestones.
4. `docs/research/landscape.md`: Rune and IronRDP findings with source paths.

## Layout

- `crates/bezel-core`: no UI, no network. The Library (`bezel.toml`), the Trust decision and certificate details, key mapping, usernames and addresses. Unit tested.
- `crates/bezel-rdp`: the Session engine. The only crate that names IronRDP types; its interface is `Session::connect`, `SessionHandle` and the `Event` channel.
- `crates/bezel`: the Rune app. `platform.rs` holds every macOS-specific path.

## Contracts

Changing any of these breaks existing users or the spec:

- The `bezel.toml` format in the spec's Data section: written atomically under a lock file, each change applied to the file as it is on disk then, never overwritten when it fails to parse, with unknown tables carried through a save.
- Keychain items with service `io.github.maxanderson95.bezel.<credential id>` and account `<credential id>`: one item per service, as `rune::Credentials` keeps them (`crates/bezel/src/keychain.rs`).
- Rune and IronRDP revisions: one pin each in the workspace `Cargo.toml`. A Rune bump also updates every `[patch.crates-io]` entry to the same rev from Rune's own `Cargo.toml`; an IronRDP bump also updates `IRONRDP_REVISION` in `crates/bezel-rdp/src/lib.rs`.

## Develop and verify

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p bezel                                  # the app
target/debug/bezel --list-scenes                    # fixture scenes
target/debug/bezel --snap out.png --scene certificate --theme dark --scale 2 --settle
BEZEL_UPDATE_GOLDENS=1 cargo test -p bezel --test goldens   # after an intended visual change
scripts/bundle.sh                                   # Bezel.app for this Mac
scripts/icon.sh                                     # assets/Bezel.icns from assets/bezel.svg
```

Views use `rune-kit` components and `rune-theme-opencode` tokens only: no colour literals. Render every view you touch with `--snap` in light and dark. Every scene has goldens in light and dark (`crates/bezel/tests/golden`), compared by `cargo test`; a new scene fails until its goldens are written, and every changed golden is looked at before it is committed. Renders that miss land in `target/tmp/golden-actual`, which CI uploads as an artifact. Behaviour against a real Windows host is checked by hand with `docs/testing.md`.
