# Bezel research: landscape

Research date: 2026-11-25. Sources read at these revisions:

| Source | Ref | Notes |
|---|---|---|
| Rune (`anomalyco/rune`) | `dev` @ `ee84b9c8` (2026-11-25) | Local clone `~/.btca/agent/sandbox/rune`, worktree `rune@ee84b9c8` |
| IronRDP (`Devolutions/IronRDP`) | `master` @ `a0ad31cb` (2026-11-25) | Worktree `IronRDP@a0ad31cb` |
| Spike | `~/.t3/serve/scratch/2026-11-25-i-want-to-do-a-341489c5` | Pins Rune `a02609f0`, IronRDP `0329c499` (both ancestors of the refs above) |
| Hex (`anomalyco/hex`) | `d0b6d87f` | Shipping Rune app |
| OpenCode (`anomalyco/opencode`) | `dev` | `packages/ui`, `packages/tui` themes |
| Transom (`jm2/transom`) | `f65c839` | Closest existing competitor |

Citations use `repo@ref:path:line`. "Inference" marks a conclusion I did not verify against source or a primary doc.

## 1. The prior spike

### Stack

- RDP: IronRDP's high-level engine crate `ironrdp-client` (feature `rustls`), plus `ironrdp-input`, `ironrdp-pdu`, `ironrdp-tls`. No FreeRDP. (`spike:Cargo.toml`)
- UI: Rune with the `tooling` feature, from a pinned git checkout in `.deps/`.
- Async: a dedicated `rdp-session` OS thread owning a 2-worker tokio runtime that runs `RdpClient::run()` and a forwarder task. (`spike:src/session.rs:91-152`)
- Packaging: `build.sh` fetches pinned revisions, builds a `prototype` profile, assembles `Rune RDP.app`, ad-hoc signs it. `Info.plist` sets `NSHighResolutionCapable`.

### Rune was not modified

The `[patch.crates-io]` block in the spike's `Cargo.toml` (winit, sctk-adwaita, parley_engine, wgpu-hal, naga, accesskit_*) points at Rune's own `vendor/` directory. That is Rune's documented requirement for every consumer, because Cargo does not inherit patches from a dependency (`rune@ee84b9c8:README.md`, "Add it"; `crates/rune/README.md:3848-3876`). `git status` in `.deps/rune` is clean. The `flush_core`, `NeedsGpuAction` and `classify_needs_gpu` warnings are dead-code warnings inside Rune itself at that revision, not something the spike introduced. Harmless.

### Frame path (IronRDP to screen)

1. IronRDP emits `RdpOutputEvent::Image { buffer: Vec<u32>, width, height }`, a full-framebuffer snapshot of `0x00RRGGBB` pixels, on a bounded tokio mpsc channel (`spike:src/session.rs:110-125`).
2. The forwarder converts every pixel to RGBA bytes (a full new `Vec<u8>` per frame), stores it in an `Arc<Mutex<Option<Frame>>>` (latest wins) and calls Rune's `Waker`.
3. Rune's `poll` hook (`AppSpec::poll`) runs on the UI thread, takes the frame and calls `Pixels::set` (`spike:src/main.rs:245-249`).
4. The view draws it with `el().size(w*scale, h*scale).pixels(&s.pixels)` inside a black, letterboxed "stage" (`spike:src/main.rs:657-662`).

Two full-frame copies happen per update (u32 to RGBA, then Rune's upload). Rune's `Pixels::set` is thread-safe and wakes only windows showing the image by itself (`rune@ee84b9c8:crates/rune/README.md:2906-2916`), so the mutex/poll hop is unnecessary.

### Input

- Keyboard: `El::on_physical_input` on the focused desktop element receives `PhysicalInput::{Key, Modifiers, FocusLost}` before Rune's own editing and focus handling (`rune@ee84b9c8:crates/rune/src/input/physical_input.rs:8-48`). The spike maps `PhysicalKeyCode` to PC scancodes with a hand-written US table, feeds them through `ironrdp_input::Database` to get press/release diffs, and sends `RdpInputEvent::FastPath` batches (`spike:src/session.rs:183-347`). `FocusLost` releases all held keys.
- Modifiers: left and right are collapsed to one side (comment at `session.rs:340`). Cmd maps to the Windows key (`0xe05b`).
- Mouse: `on_pointer_move_local` (with `tx.live_only()` so moves skip the view rebuild), `on_drag` for left button, `on_secondary_press_local` for right click (sent as press+release, so no right-drag), `on_wheel` for vertical wheel only (`spike:src/main.rs:664-706`).

### Resize and HiDPI

On every drawn layout change the spike schedules a resize 250 ms out (`window_layout`, `main.rs:202-209`); `poll` then sends `RdpInputEvent::Resize { scale_factor: 100, .. }` with one remote pixel per Mac point (`main.rs:18-25`, `session.rs:165-181`). IronRDP uses Display Control in-session and falls back to a reconnect (`DisplayResizeFallback`), which the spike surfaces as "Reconnecting at the new size…". The assistant verified on loopback that one drag produced exactly one resize request; the test server has a fixed desktop, so in-session resizing on a real Windows host was not verified in that session.

### Storage and certificates

- `library.json` in `~/Library/Application Support/Rune RDP Prototype` holds devices and usernames; passwords are Keychain items via `rune::credentials::Credentials::system()` keyed by a per-credential service name (`spike:src/library.rs`, `main.rs:126-147`).
- Certificate prompt: the IronRDP validation callback (invoked only when strict validation fails) blocks the TLS thread on an mpsc reply for up to 90 s while the UI shows the SHA-256 fingerprint with **Trust once**. Nothing is persisted (`session.rs:60-68`, `main.rs:599-640`).

### What worked

Per the session transcript (read from T3's `statev2.sqlite`, since OpenCode's archive holds no assistant text for this T3 thread): build succeeded after one dependency fix, a loopback test against IronRDP's example server completed CredSSP, received a decoded frame and logged keyboard and mouse events, a Keychain password survived an app restart, and Max reported "works!" against a real host.

### Hacky or broken

| Issue | Where |
|---|---|
| Retina is not handled: remote desktop is requested at point size with `scale_factor: 100`, then upscaled 2x, so text is soft on Retina | `main.rs:18-25`, `session.rs:173` |
| Full-frame `Vec<u32>` to `Vec<u8>` conversion and copy per update; IronRDP's dirty-rect mode (`with_desktop_updates`) unused | `session.rs:110-125` |
| Keychain calls on the UI thread; Rune documents they block and may prompt, and should run off-thread | `main.rs:137-166`; `rune:crates/rune/README.md:2669` |
| Certificate decision blocks a runtime thread with a 90 s timeout; no persistence | `session.rs:60-68` |
| Remote cursor ignored (`PointerBitmap`, `PointerHidden` dropped); local arrow always shown | `session.rs:142` |
| Scancode table is US-only; left/right modifiers collapsed; no Ctrl+Alt+Del | `session.rs:239-347` |
| Wheel sends `e.dy.round()` as rotation units; RDP wheel units are 120 per notch (inference: scrolling speed will be wrong) | `main.rs:692-699` |
| One session per app; desktop and library share one window and one `State` | `main.rs:37-56` |
| Input dropped silently when the channel is full (`try_reserve` failure) | `session.rs:183-198` |
| Client name and platform hard-coded (`RuneRDP`, `MACINTOSH`) | `session.rs:71-74` |

### Cross-platform answer given to Max

"Yes. Keep Rune and IronRDP if Windows and Linux are goals." The protocol and UI code is portable Rust; `rune::credentials` already maps to Keychain, Credential Manager and Secret Service. The Mac-specific parts are small: the `~/Library` data path, the `MACINTOSH` platform constant, and `.app` packaging. Budget time for behavior testing: Alt/Win shortcuts and focus, mixed-DPI resizing, Linux display servers and keyrings. Recommendation was native builds on all three OSes from day one, with a first milestone of save, connect, type, resize, reconnect with a saved password.

### Naming note from the same session

The assistant flagged after research that "Bezel" collides with an existing Mac screen-mirroring app, "Sash" with a Mac window manager (getsash.app), and "Transom" with `jm2/transom`, a Rust RDP client. Max chose Bezel anyway; worth a trademark check before any public release.

### Reusable from the spike

- The `Session` shape: IronRDP on its own thread and runtime, notices plus a latest-frame slot, `Waker` to the UI.
- The `on_physical_input` + `ironrdp_input::Database` pattern, and the `FocusLost` release.
- Library-in-JSON plus secrets-in-OS-store split, with rollback if the JSON write fails after the secret write.
- `tests/smoke.py`: drives the real app against IronRDP's example server on loopback. Worth keeping as an integration test.

## 2. Rune

### What it is

A GPU-rendered Rust UI framework "for macOS, Windows, Linux, iOS and the web, with its own renderer, text, layout and springs" (`rune@ee84b9c8:README.md:3`). Anomaly builds it to recreate OpenCode Desktop natively; it bakes in no brand and ships OpenCode's look as a separate theme crate (`PLAN.md:6-22`).

| Area | Finding | Source |
|---|---|---|
| Rendering | One instanced "uber-quad" pipeline of 128-byte instances. Native Metal backend by default on macOS; wgpu (D3D12, Vulkan, WebGPU) elsewhere; CPU painter fallback on Linux without Vulkan | `ARCHITECTURE.md:122-140`, `crates/rune/README.md:3884-3889` |
| Threading | Single UI thread does input, `poll`, view, layout, animation and paint. Background work via `rune::task`, `Latest`, `Waker` | `ARCHITECTURE.md:59-71` |
| Layout | Taffy flexbox/grid with Rune defaults (`display: flex`, `flex-shrink: 0`, border-box) | `crates/rune/README.md:17-47` |
| State | Keyed retained tree rebuilt from `fn view(&S, &mut Cx<S>) -> El`; handlers get `&mut S`; hover/press/animation never rerun the view | `crates/rune/README.md:10-14` |
| Text | Parley shaping, embedded Inter, CoreGraphics-matched rasterization on macOS, DirectWrite mode on Windows | `crates/rune/README.md:139-198` |
| Components | `rune-kit`: Button, IconButton, Switch, SegmentedControl, Select, Combobox, Menu, Popover, TextInput/TextArea, Badge, Progress, Tree, Table, Markdown, Diff, and an app shell (`SplitView`, `Sidebar`, `Toolbar`, inspector pane) | `crates/rune-kit/README.md:82-400`, `hex:docs/design-system.md:7-10` |
| Accessibility | AccessKit on all three desktops | `crates/rune/README.md` "Differences from PLAN.md" |
| Headless tooling | `--snap`, `--tree`, `--script`: render PNGs and drive input without a window | `ARCHITECTURE.md:120` |

### Platform status and maturity

- Windows and Linux are first-class targets, not ports (`PLAN.md:12-14`). Hex, a shipping Anomaly product, runs Rune on "Mac, Windows and Linux" (`hex:README.md:3`); other Anomaly Rune apps are `opencode-rune`, `spaceman` (macOS WM) and `deadbolt` (a Rune password manager, alpha).
- Linux needs `libxkbcommon`/`libwayland` dev packages at build and `libxkbcommon-x11` at runtime (`crates/rune/README.md:3877-3899`).
- Windows/Linux custom titlebars work (Rune draws caption buttons on Windows); traffic-light placement is macOS only (`crates/rune/README.md:1649-1673`).
- Maturity: pre-release. Consumers pin a `dev` revision (never rewritten). No crates.io release of record: `rune-ui` 0.0.0-dev snapshots are mentioned but the crate does not resolve on crates.io today. The project takes no pull requests, only issues (`README.md`, "Contributing"). Public history starts 2026-10-22 with about 3,400 commits since, about 2,800 in the last month. API churn is expected; the spike's rev, pinned this morning, already sits 23 commits behind `dev`.
- CI: macOS tests run on a 6-hour schedule; Windows/Linux checks live in apps like Hex (`PLAN.md:515-518`).

### Drawing a live framebuffer at 30-60 fps

Three paths exist, in rising order of effort:

1. **`El::pixels(&Pixels)`** (what the spike used). `Pixels::set(w, h, rgba)` from any thread; same-size updates reuse their GPU slot and do a row copy upload, no mips, no reallocation; `Pixels::replace` hands back the previous `Vec` for reuse. Limit 8192 px per axis. Full frame only: no dirty-rect API (`crates/rune-image/src/pixels.rs:146-376`, `crates/rune/README.md:2904-2921`). `pixels_fit(SurfaceFit::Fit)` letterboxes on whole device pixels; `image_filter(ImageFilter::Nearest)` gives exact 1:1 sampling (`README.md:2979-2991`).
2. **`NativeSurface`**: latest-wins slot for retained native frames, zero-copy on macOS via `CVMetalTextureCache` (BGRA and NV12). Designed for video, so it pairs with a VideoToolbox H.264 decoder (`crates/rune/README.md:3037-3060`). ARCHITECTURE lists Windows D3D shared textures and Linux dmabufs for `rune-media` native surfaces (`ARCHITECTURE.md:50`); I did not verify `NativeSurface` coverage on those two.
3. **Custom `Compositor` + `El::effect` slots**: the app takes over frame encoding with direct `wgpu::Device`/`Queue` access and paints inside an effect slot that Rune positions, clips and fades (`crates/rune/src/compose.rs:9-110`). This is where a persistent desktop texture with dirty-rect `queue.write_texture` uploads would live.

Inference: at 5K Retina (5120x2880) a full RGBA frame is about 59 MB, so path 1 costs roughly 1.7 GB/s of copies at 30 fps before conversion. Fine for 1080p-class windows, wasteful for full-screen Retina. Path 3 with IronRDP's `DesktopUpdate` dirty regions is the efficient path; path 2 matters once H.264 is decoded by VideoToolbox.

### Keyboard capture

`on_physical_input` sees physical key code, mods, repeat, produced text and consumed mods, and runs before logical editing, shortcuts and focus navigation; returning `true` consumes the event (`physical_input.rs:8-48`). Caveats from the key-routing doc: global shortcuts and native menu key equivalents are handled before the window sees a key (`crates/rune/README.md:335-337`), so Bezel's menu must not claim keys it wants to forward. `App::modifier_keys()` delivers lone modifier presses with side (`README.md:679-685`). OS-reserved chords (Cmd+Tab, Cmd+Space) never reach any app (general macOS behavior, not verified in Rune source). `.pointer_lock` exists for relative mouse (`README.md:604-618`).

### Windows, menus, macOS integration

- Multiple windows: `run_windows`, `App::window`, and runtime `rune::open_window(WindowSpec)` with per-window state type, focus and clocks (`crates/rune/src/window/spec.rs:1097-1111`, `README.md:2793-2902`). One window per session is supported. Native macOS window tabs: not found in source (unknown).
- Chrome: `Chrome { full_size_content, traffic_lights: Some((x, y)) }`, `Titlebar::new(height)` with drag regions, `Cx::is_full_screen()`, `Tx::toggle_full_screen()` (`README.md:1649-1673`).
- Menus: `App::standard_menus()` plus custom `MenuItem`s with submenus and closures (`README.md:937-950`).
- Appearance: `ColorScheme::{System, Light, Dark}` per window or app-wide (`crates/rune/README.md:927-936`).
- Credentials: built into Rune, not a separate crate. `Credentials::system()` maps to Keychain, Credential Manager and Secret Service; blocking, run off-thread (`README.md:2667-2686`).
- HiDPI: "The Retina scale is applied throughout"; `Tx::scale()` reports device pixels per point; `on_layout`/`on_drawn_layout` refire on scale change (`README.md:138`, `797-799`).

### Theming

Core tokens in `crates/rune-core/src/tok.rs` (OKLCH neutral dark/light, six accents, space 4/8/12/24, radius 6/8/12). The real look comes from `rune-kit` tokens filled by `rune-theme-opencode` (section 4).

## 3. Rust RDP stack: IronRDP

### Shape

Sans-I/O core (connector and session state machines, no I/O), with `ironrdp-tokio`/`ironrdp-blocking` adapters and a high-level `ironrdp-client` engine ("Portable RDP client engine without GPU acceleration", winit-agnostic, events on a bounded tokio mpsc channel) (`IronRDP@a0ad31cb:README.md:17-23`, `crates/ironrdp-client/README.md`). Devolutions uses it in its own products and ships `ironrdp-viewer` (winit + softbuffer). `ironrdp-client/src/rdp.rs` alone is about 8,300 lines. crates.io has `ironrdp` 0.17.0 and `ironrdp-client` 0.1.0 (both 2026-07-10); `master` is far ahead, so Bezel would pin a git rev like the spike.

### Features relevant to Bezel

| Feature | Status | Source |
|---|---|---|
| NLA / CredSSP, NTLM and Kerberos, KDC proxy | Yes | `README.md:27-33`; latest commit adds CredSSP package choice |
| TLS 1.2/1.3, rustls or native-tls | Yes | `crates/ironrdp-tls` |
| Cert inspection for TOFU pinning | Yes (rustls only). Callback gets leaf DER, endpoint, error string; runs only after strict platform validation fails; returning `true` accepts. Requires `CertificateValidation::Strict`. Default policy is `DangerouslyAcceptInvalidCertificate` | `crates/ironrdp-tls/src/lib.rs:49-75`, `rustls_verifier.rs:57-120` |
| Dynamic resolution (MS-RDPEDISP) | Yes. `RdpInputEvent::Resize { width, height, scale_factor, physical_size }` and full `MonitorLayout`; automatic reconnect fallback with `DisplayResizeFallback` reason | `crates/ironrdp-client/src/rdp.rs:106-115, 626-639, 1016-1030` |
| Size constraints | Width even, 200-8192 both axes (`adjust_display_size`) | `crates/ironrdp-displaycontrol/src/pdu/mod.rs:630-650` |
| HiDPI | `with_desktop_scale_factor` (100-500) and `with_device_scale_factor` (only 100, 140, 180; `build` rejects others) | `crates/ironrdp-client/src/config.rs:1246-1282` |
| Dirty-region frames | `with_desktop_updates()` switches `Image` snapshots to `DesktopUpdate { buffer, region }` | `rdp.rs:157-167, 343-371, 1242-1250` |
| Codecs | Client decodes raw, interleaved RLE, planar, RemoteFX; EGFX with AVC420, AVC444, ClearCodec, Planar, RemoteFX, Alpha | `README.md:35-40`, `crates/ironrdp-egfx/src/client.rs:1187-1206` |
| H.264 decoder | Pluggable `H264DecoderFactory`. Defaults: OpenH264 (`openh264` feature, software, bundled source) or Media Foundation on Windows; **none on macOS**. No VideoToolbox decoder exists in the tree | `crates/ironrdp-client/src/h264.rs:20-60`, `crates/ironrdp-egfx-native/src/` |
| Graphics pipeline | On by default when an H.264 decoder is configured | `config.rs:1656-1665` |
| Clipboard | `clipboard` feature, native backends for macOS, Windows, Linux | `crates/ironrdp-cliprdr-native/src/{macos.rs,windows,linux}` |
| Audio | `sound` feature, cpal output and capture | `crates/ironrdp-rdpsnd-native` |
| RD Gateway | `gateway` feature (MS-TSGU) | `crates/ironrdp-client/Cargo.toml:48` |
| Entra ID auth | Experimental `entra-auth`; embedder owns browser flow and app registration; rejects cert-override callbacks | `crates/ironrdp-client/README.md` |
| `.rdp` files | `ironrdp-rdpfile` parse and write | `README.md:33` |
| Auto-reconnect | `with_auto_reconnect(n)`, cookie-based | `rdp.rs:1237` |
| Remote cursor | `PointerBitmap`, `PointerPosition`, `PointerHidden` output events | `rdp.rs:169-175` |

### FreeRDP as the alternative

| | IronRDP | FreeRDP 3 via FFI |
|---|---|---|
| Language | Rust, fuzzed core, heavy `unsafe` linting | C; Transom documents that FreeRDP and FFmpeg parse hostile server data in-process (`transom:README.md`, Security) |
| Rust bindings | Native | `freerdp-sys` 0.1.0 (FreeRDP 3, 49 downloads); `freerdp2` stale since 2022 (crates.io) |
| Built-ins | TOFU is ours to implement via the callback | `/cert:tofu`, `/dynamic-resolution`, `/scale-desktop`, `/scale-device` built in (FreeRDP `client/common/cmdline.h`) |
| H.264 | OpenH264 or Media Foundation; macOS needs our own VideoToolbox decoder | FFmpeg/OpenH264 (inference: hardware decode on macOS depends on the FFmpeg build) |
| Shipping cost | Pure Rust plus optional OpenH264 | Bundle FreeRDP, OpenSSL, FFmpeg per platform; Transom rebuilds releases on their CVEs |
| Interop record | Younger; fewer field reports | Long history against Windows, xrdp, gnome-remote-desktop |

Recommendation: stay on IronRDP. The spike proves it, and the missing pieces (VideoToolbox decode, TOFU store) are small and in Bezel's own code.

## 4. Anomaly design language

### What Anomaly makes

anoma.ly lists "Creators of OpenCode" plus experiments (Rebase, Antarctica). The GitHub org `anomalyco` ships OpenCode (TUI, desktop, web), Hex (voice dictation, Rune), `opencode-rune` (OpenCode desktop in Rune), Spaceman (macOS tiling WM), Deadbolt (P2P password manager, Rune), Nightmail, models.dev, OpenTUI, SST, OpenAuth. ("runeicons", in the btca sandbox, is an unrelated third-party icon set.)

### The source of truth

`@opencode/ui` tokens, resolved and frozen as data in `rune@ee84b9c8:themes/opencode/assets/theme.json` (regenerated by `tools/theme.mjs`) and mapped onto `rune-kit` by `themes/opencode/src/lib.rs`. Hex states its rules: kit tokens only, never a colour literal outside the theme crate (`hex:docs/design-system.md:27-35`). Bezel should do the same: `rune_theme_opencode::kit(dark)` and `rune_kit` components.

### Colour (v2 semantic tokens, hex with alpha)

| Token | Dark | Light |
|---|---|---|
| `v2-background-bg-deep` (sidebar) | `#080808` | `#fafafa` |
| `v2-background-bg-base` | `#161616` | `#ffffff` |
| `v2-background-bg-layer-01` | `#242424` | `#fafafa` |
| `v2-background-bg-layer-02` | `#2e2e2e` | `#f2f2f2` |
| `v2-background-bg-layer-03` | `#3a3a3a` | `#eeeeee` |
| `v2-background-bg-accent` | `#3b5cf6` | `#3b5cf6` |
| `v2-background-bg-button-neutral` | `#ffffff0f` | `#ffffff` |
| `v2-text-text-base` | `#fafafa` | `#161616` |
| `v2-text-text-muted` | `#aeaeae` | `#5c5c5c` |
| `v2-text-text-faint` | `#808080` | `#808080` |
| `v2-text-text-accent` | `#a2bcff` | `#3b5cf6` |
| `v2-border-border-muted` | `#ffffff14` | `#00000014` |
| `v2-border-border-base` | `#ffffff1a` | `#0000001a` |
| `v2-border-border-strong` | `#ffffff33` | `#00000033` |
| `v2-border-border-focus` | `#a2bcff` | `#7698fd` |
| `v2-icon-icon-base` | `#dbdbdb` | `#3a3a3a` |
| `v2-state-fg-success` / `-danger` / `-warning` | `#6bd586` / `#f17471` / `#f2cf76` | `#198b43` / `#b82d35` / `#cb9f34` |
| Brand "ember" step 9 (`brand()`) | `#fc533a` | `#fc533a` |

Pattern: near-neutral greys (no blue tint), one blue interactive accent, a warm red-orange brand mark reserved for logos. Borders are translucent white or black hairlines, not grey lines. The OpenCode TUI's own default theme differs (peach primary `#fab283`, steps `#0a0a0a` to `#eeeeee`; `opencode:packages/tui/src/theme/assets/opencode.json`), so "the Anomaly look" in a GUI means the v2 tokens above.

### Type, spacing, shape, motion

- Fonts: **Inter** (variable) for UI, **IBM Plex Mono** for code (`opencode:packages/ui/src/styles/theme.css:2-5`; `themes/opencode/src/lib.rs:31-38`). Weights are unusual: regular 440, medium 530, bold 670.
- Sizes: kit `ui` 13 pt/20 lh weight 440, tracking -0.04; `small` 11/12; `body` 14/21; mono 12/18 (`themes/opencode/src/lib.rs:217-230`). Web scale 13/14/16/20 px.
- Spacing base 4 px (`--spacing: 0.25rem`). Radii xs 2, sm 4, md 6, lg 8, xl 10 (`lib.rs:237`). Small radii overall; nothing pill-shaped except switches.
- Shadows: layered, low-alpha (`shadow-xs`: `#0000000a 0 1px 2px -0.5px, #00000006 0 0.5px 1.5px, #0000000d 0 1px 3px`), often combined with a 1 px ring as the border (`theme.json` `.shadows`).
- Motion: critically damped springs, control 0.2 s, overlay 0.24 s, disclosure 0.3 s, zero bounce (`lib.rs:238-243`). Hex makes all hover changes instant (`design-system.md:52-63`).
- Icons: in-house line icons on 16 or 20 unit view boxes, 1 px `currentColor` strokes, round or square caps, generated from `@opencode/ui`'s catalog (`themes/opencode/src/icons.rs`). A few "active" variants add a 10% opacity fill.
- Light and dark: both always supported; Hex follows the system and offers a manual override (`design-system.md:13-17`).
- Layout: "a deep-background sidebar and a raised, rounded content panel" (OpenCode Desktop, copied by Hex: `design-system.md:19-21`). `rune_kit::shell` gives the sidebar, 48 pt draggable top band clearing the traffic lights, toolbar and inspector (`crates/rune-kit/README.md:294-330`).
- Density and copy (Hex rules worth adopting): sentence case everywhere, one primary (contrast) button per screen, one to three visible actions with the rest in an overflow menu, destructive actions last with two-step confirm, hover fills only on single-target rows, row descriptions one phrase with no period (`design-system.md:39-121`).

## 5. Landscape: existing macOS clients

Prior research (sessions `ses_f6f2c2244ffeD6wRxqwaL3lzKX`, `ses_f732f09baffenuFcPrEScDU0km`, Sept 2026) listed Royal TSX, Jump Desktop, Remote Desktop Manager and Parallels Client as the established options, and FreeRDP Launcher and Selective Remote as the open-source picks; Max found Royal TSX and RDM bloated and Windows App frustrating. The table adds what I could verify today against Max's pain points.

| Client | Engine | Does well | Falls short for Max |
|---|---|---|---|
| Windows App (Microsoft) | Microsoft | Best protocol fidelity; per-device "Update the session resolution on resize" and "Optimize for Retina displays" (Microsoft notes Retina costs ~4x bandwidth) ([display settings](https://learn.microsoft.com/en-us/windows-app/display-settings)); AVD/W365/Entra | Password edits need Keychain by hand (Max); one Quick Connect window at a time, a second disconnects the first (Max, and a Microsoft Tech Community post on 11.0.4); large square device tiles (Max) |
| Royal TSX | FreeRDP 3 | Most feature-complete; reusable credential objects; per-connection Display Options with "Use full Retina resolution", `ScaleFactor`, `SmartReconnect` (reconnects on resize), `SmartSizing` ([RoyalRDSConnection](https://docs.royalapps.com/products/scripting/objects/connections/royalrdsconnection.html)) | Retina off by default (`UseRetinaResolution=false`); resize means reconnect; heavy document/tree UI; freemium connection cap |
| Jump Desktop | Own RDP + Fluid | Polished native Mac app; "Match Display Resolution" and "Use Retina Resolution" (documented for Fluid sessions) | Best features need its Fluid host agent; RDP-specific resize and cert-trust behavior not verified |
| Remote Desktop Manager | Devolutions (FreeRDP lineage) | Credential vault; "Dynamic resolution" mode without reconnect ([RDP entry docs](https://docs.devolutions.net/rdm/knowledge-base/how-to-articles/rdp-session-entry)) | Enterprise PAM tool, very dense; account/cloud tiers |
| sdl-freerdp | FreeRDP 3 | `/cert:tofu`, `+dynamic-resolution`, `/scale-desktop`, `/scale-device` | CLI only, no connection manager |
| Transom (`jm2/transom`, v0.1.0) | FreeRDP 3 + GTK 4/libadwaita, Rust | Nearly Max's exact list: pixel-exact HiDPI incl. fractional scale, dynamic resolution, saved connections, quick connect, search, TOFU with refuse-on-change, keyring passwords, detachable tabs, clipboard, audio, RD Gateway | GNOME-first GTK app on macOS; ad-hoc signed; FreeRDP/FFmpeg in-process (planned helper and IronRDP engine not built); prior session found no malware but would not use it with production creds |
| RDP Manager (`patrikcze/rdp-manager`, 0.4.2) | Swift "RDPKit" + FreeRDP 3 | SwiftUI/AppKit; Cmd+K quick connect, many concurrent tabbed sessions, resolution follows window at Retina scale, VideoToolbox H.264/AVC444, Keychain, cert change flagged, group inheritance of credentials | Young; admin-oriented (groups, SSH); feature-dense |
| Selective Remote (v0.31.0) | SDL-FreeRDP | Free, MIT, native; Retina-aware, multi-monitor RDP, Keychain | SSH/SFTP/tunnel workspace, broader than RDP; ad-hoc signed |
| FreeRDP Launcher | SwiftUI wrapper for sdl-freerdp | Small native manager | Launches an external SDL window per session; no integrated desktop |

### UI patterns worth stealing

- Cmd+K quick connect that accepts `host`, `host:port` or IP, with "save this connection" afterwards (RDP Manager).
- Credentials as first-class objects that connections reference, editable in place, with groups setting inherited defaults and the UI showing where a value comes from (RDP Manager, Royal TSX).
- TOFU that is quiet on first use for self-signed hosts, loud on change, with "cancel" as the default action and "accept" styled as dangerous (Transom security doc).
- Per-connection scale mode: match local (default), fixed percent, legacy 100% stretched for servers that ignore scale (Transom `PLAN.md:88-95`).
- Separate "Disconnect" from "Sign out of Windows…", and Ctrl+Alt+Del from a menu (RDP Manager).
- Compact list rows: name, host, credential label on one or two lines, search over everything; Hex's `list_row` and `source_list` are the Anomaly-styled equivalent.

## Implications for Bezel

Decisions to make:

1. **Framebuffer path.** Start with `El::pixels` plus `Pixels::replace` buffer reuse and set frames straight from the RDP thread (drop the mutex/poll hop). Plan a move to a `Compositor` effect slot that owns a persistent texture and applies IronRDP `DesktopUpdate` dirty rects, before full-screen 5K Retina becomes the default case. Measure first; the spike's copies may be fine for windowed use.
2. **HiDPI.** Request the desktop in device pixels (points x backing scale, width even, 200-8192), send desktop scale factor `round(scale*100)` and device scale factor nearest of 100/140/180, and draw 1:1 with nearest filtering. This is Transom's recipe and matches IronRDP's constraints. Decide whether a "legacy 100%" per-connection fallback is needed for bandwidth-limited links.
3. **Codec.** Without an H.264 decoder IronRDP stays on RemoteFX/legacy bitmaps. Options: OpenH264 (software, bundled source; my understanding is Cisco's patent coverage applies only to its own prebuilt binaries, so verify licensing), or write a VideoToolbox `H264Decoder` and feed `NativeSurface` zero-copy. Defer until bitmap performance is measured.
4. **Cert pinning.** Use `CertificateValidation::Strict` plus the callback: valid chain passes silently; otherwise compare SHA-256 of the leaf DER against a per-host pin in the library; unknown host prompts once with "Trust" persisted; a changed pin is refused unless explicitly accepted. Requires the rustls backend. The callback is synchronous inside the handshake, so the prompt round-trip needs a cleaner design than the spike's 90 s blocking wait.
5. **Credential storage.** `rune::credentials` already covers Keychain, Credential Manager and Secret Service (not Linux without D-Bus). Keep secrets keyed by credential ID, metadata in Bezel's own file, all calls off the UI thread. Editing a password is then a `write` to the same service name, which fixes the Windows App complaint directly.
6. **Sessions and windows.** One `open_window` per session gives multiple simultaneous quick connects. Native macOS window tabs are not visibly supported in Rune (unknown); an in-app tab strip is the fallback.
7. **Keyboard.** Need a layout-correct scancode map (or send Unicode for text), left/right modifiers, a Cmd mapping policy, Ctrl+Alt+Del, and menu key equivalents that do not swallow forwarded chords.

Risks:

- Rune is about five weeks into public history, pre-release, accepts no PRs, and churns daily. Pin revs, bump deliberately, keep Bezel's Rune surface narrow, and file issues upstream.
- IronRDP `master` also moves fast and the crates.io release lags months behind; pin a git rev.
- Interop against real Windows hosts (Display Control resize in place, Kerberos, Gateway) is the least verified part of the spike. Build a manual test matrix early.
- Windows and Linux builds are unverified for this code; CI on all three from day one, as the spike session recommended.
- "Bezel" collides with an existing Mac screen-mirroring app; check before publishing.
