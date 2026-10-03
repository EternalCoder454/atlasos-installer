# Atlas.Ui (copy)

The shared look of Atlas apps, copied from
[atlasos-updater](https://github.com/EternalCoder454/atlasos-updater)'s `ui/`
at d6ba3d0 so the installer builds on its own (the ISO build and the RPM have
no access to the updater's tree).

Added here, to move upstream so other Atlas apps can use them:

- `StepItem.qml`: one step in a setup sidebar (done, current or to come)
- `AtlasProgressBar.qml`: a rounded accent progress bar

Keep the other files identical to upstream; change them there first.
