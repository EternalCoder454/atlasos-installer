# First-start apps

Installs the apps picked on the installer's "Choose your apps" page. The
installer writes `/var/lib/telamon/first-boot-apps.json`
(`{"version": 1, "apps": ["firefox", "gh"]}`); ids are looked up in the catalog,
`apps.json` (shipped as `/usr/share/telamon/first-boot-apps.json`). Nothing from
the record is executed or used as a URL.

## What runs when

- `telamon-first-boot-apps.service` (system, root, after network-online): installs
  `flatpak:` entries system-wide from Flathub, all in one `flatpak install`
  transaction (one dependency solve and one download queue; a single app is
  installed on its own). When that fails, it goes through the apps one at a
  time with `--or-update`, to find which one failed and to keep the ones that
  went in. It writes progress to
  `/var/lib/telamon/first-boot-apps.status`, then rewrites the record with only
  the per-account entries (or deletes it). Fails non-zero on error, so systemd
  retries with backoff.
- `telamon-first-boot-apps.service` (user, at graphical login): installs `mise`
  (pinned release, SHA-256 checked, to `~/.local/bin/mise`), `mise:<tool>` via
  `mise use -g`, and `toolbox:<packages>` into the default toolbox. Done ids go
  in `~/.local/state/telamon/first-boot-apps.done`. `mise` tools and the
  toolbox install at the same time (they use different parts of the machine
  and the network), each retrying on its own. Shows notifications for
  both parts. Retries in-process with backoff (30 s up to 30 min) while offline.
- `/etc/profile.d/telamon-mise.sh` activates mise in bash when it is installed.

## The names until the rename (atlas-first-boot-apps)

This was `atlas-first-boot-apps`, under `/var/lib/atlasos` and
`~/.local/state/atlasos`. For one release the old names work, because the
image's presets and checks and installed systems still name them:

- The installer writes the record under **both** `/var/lib/telamon` and
  `/var/lib/atlasos` (the same list), since the image decides which script runs.
  The script reads the new one, else the old one; when it saves, it writes the
  new one and removes the old one, so what is left to do is in one place.
- A done marker or a result under the old name counts as done: the user's
  `~/.local/state/atlasos/first-boot-apps.done` and `.notified`, and the status
  file, are read together with the new ones. A machine whose first start
  finished under the old name does not run it again; a half-done one carries on.
- The units start when either record exists. `install.sh` also installs
  `atlas-first-boot-apps.service` (system and user) as a link to the new unit
  (that is what an alias is to systemd), the old wants links, and links from
  `/usr/libexec/atlasos/atlas-first-boot-apps` and
  `/usr/share/atlasos/first-boot-apps.json`. `/etc/profile.d/atlas-mise.sh` is
  an empty stand-in, so mise is activated once.

## Files

`telamon-first-boot-apps` (script), `units/`, `profile.d/`, `apps.json` (catalog),
`install.sh DESTDIR` (the image build runs `firstboot/install.sh /`; it also
creates the enablement symlinks), `tests/`.

## Add an app

Add one line to `apps.json` with an `install` of `flatpak:<app id>`, `mise`,
`mise:<tool>` or `toolbox:<packages>`. `gh` and `ollama` come through mise's registry
(the `aqua` backend; see "What is downloaded") at their latest version; only
mise itself is pinned and hashed. To bump mise, change `MISE_VERSION` and
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

## What is downloaded, from where, and how it is checked

Nothing in the record is executed or used as a URL: its ids are looked up in
the catalog (root-owned, in the image), and the lookup's result is matched
against a strict pattern before it reaches a command line (a flatpak id, a
mise tool name or a package name never starts with `-`). Both modes refuse a
record that is not root-owned or is group- or world-writable.

| What | From | Checked by |
|---|---|---|
| Flathub remote | `https://dl.flathub.org/repo/flathub.flatpakrepo` (`flatpak remote-add --if-not-exists`) | TLS; the remote's GPG key comes in that file, and flatpak checks the signature of every commit it installs |
| Flatpak apps and add-ons (`com.jeffser.Alpaca` and its `Plugins.Ollama` and `Plugins.AMD`, browsers, VS Code) | Flathub | the OSTree commit signature (Flathub's key), whose build recipe pins the sha256 of every source. Flathub is a community store: the add-ons are the apps' authors' builds, not Telamon's or Fedora's |
| mise | `https://github.com/jdx/mise/releases/download/<MISE_VERSION>/` | `curl --proto =https --tlsv1.2`, a size limit, and the pinned SHA-256 (`MISE_SHA256`), checked before the file is made executable |
| `gh` (`mise use -g gh`) | GitHub release of `cli/cli`, through mise's `aqua` backend | the release's digest from GitHub, and the GitHub artifact attestation (a signed build provenance), which mise verifies; a failure to reach the attestation service stops the install |
| `ollama` (`mise use -g ollama`) | GitHub release of `ollama/ollama`, through the `aqua` backend | only the digest GitHub lists for the file (it detects a damaged download, not a malicious release): ollama publishes no signature or attestation. TLS to github.com is the trust anchor |
| Toolbox image | `registry.fedoraproject.org/fedora-toolbox:<release>` (`toolbox create`) | TLS and the system's container policy, which accepts images outside Telamon OS's own without a signature |
| Toolbox packages (`gdb strace perf`) | Fedora's repositories, `dnf install` in the toolbox | `gpgcheck` of the repositories |

`gh` and `ollama` are installed at their latest version (`latest` in the
account's mise configuration), so a new install gets what upstream has
published that day; only mise itself is pinned. When mise runs, its
environment (`MISE_ENV` in the script) turns off every backend that runs
third-party code (`asdf` and `vfox` plugins, `cargo`, `go`, `npm`, `pypi`,
`gem`, ...: `mise registry gh` lists `aqua` and then an `asdf` plugin, and the
plugin is a git repository whose scripts would run as the account) and sets
the verification switches (cosign, SLSA, minisign, GitHub attestations,
fail when the attestation service cannot be reached) to on, so a setting in the
account's own environment cannot weaken them for this install. A tool for the
catalog must therefore come from `aqua` (or a mise core tool).

## Tests

`env -u DISPLAY -u WAYLAND_DISPLAY python3 -m unittest discover -s firstboot/tests -v`
(fakes on PATH; path overrides work only with `TELAMON_FBA_TEST=1`).
