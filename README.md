# bezel

> [!WARNING]
> This project is a work in progress. It is still being built and is not ready for use.

Bezel is a small, fast RDP client for connecting from a Mac to Windows machines, written in Rust with [Rune](https://github.com/anomalyco/rune) and [IronRDP](https://github.com/Devolutions/IronRDP). The remote desktop is drawn one remote pixel per screen pixel on Retina displays and follows the window as it resizes.

## Platforms

macOS on Apple silicon or Intel. The code also builds on Linux so CI keeps it portable, but only macOS is supported.

## Install

Bezel is installed by building it from source. With Rust installed through rustup (the pinned toolchain in `rust-toolchain.toml` installs itself):

```sh
scripts/bundle.sh
open target/release/Bezel.app
```

`scripts/bundle.sh` builds a release binary and wraps it in `Bezel.app` with an ad-hoc signature for this Mac. `cargo run -p bezel` runs it without a bundle.

## Use

Type a host name (`dc01.corp.lan`, `dc01:3390` or an IP address) and press Return, then sign in with `DOMAIN\user`, `user@domain` or a local `user` and its password. Bezel signs in with Kerberos when it can find a domain controller, NTLM otherwise, and shows the Windows login screen on servers with Network Level Authentication off.

Save servers with + at the foot of the sidebar (New connection, New group). Clicking a Connection signs in with its Credential, or asks for a username and password when it is set to "Ask each time". Right-click a Connection or Group for Edit, Duplicate, Move to group, Rename and Delete, or drag Connections between Groups and Groups into a new order. ⌃⌘S hides the sidebar.

A Credential is a named username and password that any number of Connections share. When a password changes, change it once under Credentials, or type the new one where the failed sign-in asks for it, and every Connection using that Credential signs in with it.

When a server's certificate doesn't come from an authority the Mac trusts, Bezel shows its fingerprint and asks. Tick "Remember this certificate" to be asked only if it changes. Certificates in the sidebar lists the remembered ones and removes them.

⌘ plus a key reaches Windows as Ctrl plus that key, so ⌘C and ⌘V copy and paste; ⌘ pressed alone is the Windows key. ⌘W disconnects.

## Files

- `~/Library/Application Support/Bezel/bezel.toml` holds Connections, Groups, Credential names and usernames, settings and remembered certificates. It can be edited by hand; Bezel never overwrites a copy it can't read.
- Passwords live only in the Keychain, one item per Credential named `io.github.maxanderson95.bezel.<credential id>`.
- `~/Library/Logs/Bezel` holds a daily log, kept for a week, with no passwords in it. "Copy details" under a connection error copies what the log would need.

## License

MIT
