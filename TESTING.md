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
    - [ ] A started session captures the selected output and injects only selected devices
    - [ ] Closing the session stops capture and disconnects input, including a client-held EIS fd
