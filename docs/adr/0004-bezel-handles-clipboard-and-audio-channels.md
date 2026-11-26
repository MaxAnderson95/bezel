# Bezel handles the clipboard and audio channels itself

IronRDP's client offers a clipboard and audio for each platform, but Bezel registers its own handlers for both channels inside `bezel-rdp`. IronRDP's macOS clipboard bridge must be created, polled and dropped on the main thread, which would put an IronRDP type in the app, and it also carries images and files, which v1 leaves out. Its sound channel offers no way to silence one Session. Bezel's clipboard handler is text only and keeps the Mac's text in the engine, so it answers Windows from the Session's own thread, while the app reads and writes the Mac's clipboard through Rune. Its audio handler wraps IronRDP's playback and drops wave blocks while the Session is muted.

## Consequences

- Images and files on the clipboard stay unshared until Bezel extends its handler.
- Audio uses the static RDPSND channel only, without the dynamic audio channels Windows 8 and later prefer; the manual checklist confirms Windows plays through it.
- IronRDP's playback is built without its Opus decoder, whose build needs CMake.
