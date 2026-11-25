# IronRDP as the RDP engine

Bezel uses IronRDP, pinned to a git revision, rather than FreeRDP 3 through FFI. IronRDP is pure Rust, the prior spike already drives it from Rune, and it exposes dynamic resize, Retina scale factors and a certificate callback Bezel can build its own trust store on. FreeRDP has the longer interop record and built-in certificate trust, but would mean bundling FreeRDP, OpenSSL and FFmpeg per platform, relying on immature Rust bindings, and parsing hostile server data in C.

## Consequences

- crates.io releases lag `master` by months, so Bezel pins a git revision and bumps it deliberately.
- IronRDP has no VideoToolbox H.264 decoder; until Bezel writes one, sessions use RemoteFX and bitmap codecs.
