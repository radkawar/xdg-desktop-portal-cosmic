# Testing

This document provides a regression testing checklist for the COSMIC XDG desktop portal. The checklist provides a starting point for Quality Assurance reviews.

## Checklist

- [ ] Screenshots work
    - [ ] GUI interaction works normally with 100% or non-100% scaling
    - [ ] Rotate a screen; screenshot GUI appears correctly & screenshot of that screen has correct orientation
    - [ ] Screenshot files are saved
    - [ ] Latest screenshot's copied to clipboard
- [ ] PipeWire screen capture from OBS with the portal prompt works
- [ ] Webcam and screen sharing prompted through Firefox works
- [ ] The file chooser works from Firefox
- [ ] The file chooser, webcam, and screen share work from the Slack flatpak
- [ ] RemoteDesktop asks for consent before enabling input
    - [ ] `ConnectToEIS` and legacy `Notify*` cannot inject before successful `Start`
    - [ ] Cancelling consent grants neither capture nor input
    - [ ] Cancelling startup leaves no worker, even before formats or node ID arrive
    - [ ] Native previews and capture startup progress after format announcements
    - [ ] The first legacy input event reaches resumed EI devices without warm-up
    - [ ] Legacy input initialization and event delivery preserve D-Bus wire order
    - [ ] A started session captures the selected output and injects only selected devices
    - [ ] Closing the session stops capture and disconnects input, including a client-held EIS fd

## Native RemoteDesktop lifetime regression

Build `cargo build --locked --example remote_desktop_lifetime` and run the
example on a real COSMIC session. It uses the public portals and actual local
consent; it never grants access by calling a backend method directly.
The capture check needs GStreamer's PipeWire, base and good plugins. Use
`--frame-output <path.png>` to retain the actual captured pixels as evidence.

On Pop!_OS, pin a coherent current COSMIC/PipeWire package set rather than
combining a newer portal with the installer's older compositor or PipeWire.
The native input reader must also use the current terminal package: the
installer's older terminal can lose initial symbols during focus changes.
Record exact package versions and the tested backend binary hash.

- Run `remote_desktop_lifetime` and select one display in the native dialog.
  The example requires a real PipeWire video buffer, then waits at `CLOSE_READY`.
  Record `pw-dump` and confirm the exact printed `NODE` exists. Continue on stdin.
  After `CLOSED_FD_RETAINED`, record `pw-dump` again and require that node to be
  absent while the client and its original EIS fd remain alive. Only then
  continue to the final `PASS`. The client also requires EOF/reset on the
  retained fd, a failed late write, and rejection of a post-close `Notify*`.
- Use `--input-text <nonce>` to additionally exercise real pointer and keyboard
  injection. At `INPUT_READY`, position the local pointer over an unfocused
  terminal reader without clicking locally or warming up `Notify*`. Continue on
  the client's stdin: the first remote click must focus the reader and the full
  nonce must arrive. The example never writes a guest proof file.
- Run `--view-only` to exercise standalone ScreenCast without requesting any
  input device. Verify the same exact-node disappearance at `CLOSED_SESSION`
  before allowing the still-connected client to exit.
- Run `--expect-cancel` (and separately `--view-only --expect-cancel`) and press
  the real native Deny/Cancel button. Only the actual cancelled response passes.
  At `CANCELLED_SESSION`, require no `cosmic-screencast` nodes in `pw-dump`
  while the client is still connected, then continue to the final `PASS`.
- Compare the retained-fd check with the unmodified backend. A client-held fd
  that stays connected after `Session.Close` must fail, not become a success
  merely because the client exits or its GStreamer pipeline stops.

