# Changelog

## 0.1.0 (unreleased)

First release.

- Starfield warp intro that settles into your distro's logo, with info lines revealed in turn.
- Logos for 22 distros plus a Tux fallback, picked from `/etc/os-release` (`ID`, then `ID_LIKE`). Override with `--logo` or `WARPFETCH_LOGO`; list them with `--list-logos`.
- System info read directly from `/proc`, `/sys` and `/etc`: OS, host, kernel, uptime, packages, shell, desktop, terminal, CPU, GPU, memory, disk.
- Package counts for rpm, dpkg, pacman, apk, portage, xbps, nix, Flatpak and Snap. rpm, xbps and nix are cached and recounted in the background when their database changes, so startup never waits on them.
- Tokyo Night colours, plus a matching Konsole colour scheme in `extras/`.
- `--no-anim`, `WARPFETCH_NO_ANIM`, `--help`, `--version`.
- `tools/logo2braille.py` for converting logos to braille art.
