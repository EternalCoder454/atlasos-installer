# First-start apps

Installs the apps picked on the installer's "Choose your apps" page. The
installer writes `/var/lib/atlasos/first-boot-apps.json`
(`{"version": 1, "apps": ["firefox", "gh"]}`); ids are looked up in the catalog,
`apps.json` (shipped as `/usr/share/atlasos/first-boot-apps.json`). Nothing from
the record is executed or used as a URL.

## What runs when

- `atlas-first-boot-apps.service` (system, root, after network-online): installs
  `flatpak:` entries system-wide from Flathub, writes progress to
  `/var/lib/atlasos/first-boot-apps.status`, then rewrites the record with only
  the per-account entries (or deletes it). Fails non-zero on error, so systemd
  retries with backoff.
- `atlas-first-boot-apps.service` (user, at graphical login): installs `mise`
  (pinned release, SHA-256 checked, to `~/.local/bin/mise`), `mise:<tool>` via
  `mise use -g`, and `toolbox:<packages>` into the default toolbox. Done ids go
  in `~/.local/state/atlasos/first-boot-apps.done`. Shows notifications for
  both parts. Retries in-process with backoff (30 s up to 30 min) while offline.
- `/etc/profile.d/atlas-mise.sh` activates mise in bash when it is installed.

## Files

`atlas-first-boot-apps` (script), `units/`, `profile.d/`, `apps.json` (catalog),
`install.sh DESTDIR` (the image build runs `firstboot/install.sh /`; it also
creates the enablement symlinks), `tests/`.

## Add an app

Add one line to `apps.json` with an `install` of `flatpak:<app id>`, `mise`,
`mise:<tool>` or `toolbox:<packages>`. `gh` comes through mise's registry
(with aqua checksum checks) at its latest version; only mise itself is
pinned and hashed. To bump mise, change `MISE_VERSION` and
`MISE_SHA256` in the script together.

## Local AI

Two ordinary entries in the `ai` group, shown together on the apps page under
a graphics-card warning: `ollama` (`mise:ollama`, the upstream release through
mise's registry, whose main archive carries the CPU, NVIDIA CUDA and Vulkan
backends) and `alpaca` (`flatpak:com.jeffser.Alpaca`, a chat app).

A `flatpak:` entry may list add-ons, installed system-wide right after the app
(if one fails the whole entry is retried): `extra` always, `extra_amd` only when
the first-start script finds an AMD display controller (PCI vendor 0x1002,
class 0x03, in `/sys/bus/pci/devices`). Alpaca uses both:
`com.jeffser.Alpaca.Plugins.Ollama` (Alpaca runs and manages its own Ollama
server, so no service is needed) and `com.jeffser.Alpaca.Plugins.AMD` (ROCm).

Nothing pulls a model: they are several GB each, so the user pulls them from
Alpaca (or `ollama pull`) later. The standalone `ollama` CLI still needs
`ollama serve`, and the archive mise fetches has no AMD ROCm backend (upstream
ships it separately); AMD cards use its Vulkan one.

## Tests

`env -u DISPLAY -u WAYLAND_DISPLAY python3 -m unittest discover -s firstboot/tests -v`
(fakes on PATH; path overrides work only with `ATLAS_FBA_TEST=1`).
