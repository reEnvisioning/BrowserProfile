# BrowserProfile

`bp` is a small standalone CLI for safe runtime Firefox and LibreWolf profile management. The Nix package also installs `browserprofile` as a symlink to `bp`.

```sh
bp create work --browser firefox --default
bp apply default --browser firefox --backup
bp list                         # Firefox and LibreWolf, browser-qualified
bp list --browser firefox       # Firefox only
bp default get --browser firefox
bp launch work --browser firefox -- https://example.org
bp remove work --browser firefox --yes
```

Profiles created by `bp` live under `$XDG_DATA_HOME/browserprofile/` (or
`~/.local/share/browserprofile`). Browser registries are read from Firefox's
`$XDG_CONFIG_HOME/mozilla/firefox/profiles.ini` and LibreWolf's
`$HOME/.librewolf/profiles.ini`. On the first upgraded LibreWolf command, bp
imports only BrowserProfile-owned registrations from the obsolete nested
`$XDG_CONFIG_HOME/librewolf/librewolf/profiles.ini`; that file is left
untouched. Names are deliberately restricted to letters, digits, `_`, `-`,
and `.` and cannot begin with `.`.

`bp launch` passes `--profile` directly to the browser, so a successful launch
does not prove that LibreWolf's persistent profile menu registry contains the
profile.

`list` without `--browser` aggregates Firefox and LibreWolf as sorted,
browser-qualified names; `--all` also includes registered profiles. `remove`
removes its exact registry entry after deletion (or cleans up a missing listed
directory) without rewriting unrelated INI sections or leaving a default entry
pointing at it. `remove` only deletes profiles carrying `.browserprofile-owned`
unless an unmanaged registered profile is explicitly confirmed. `apply`,
`default set`, and `remove` each require their own lowercase-`y` `[y|N]` prompt
for unmanaged registered profiles; each prompt identifies the resolved target
directory, and `--yes` never bypasses it. Removing a browser-default owned profile always asks `Remove browser default profile NAME?
[y|N]`, even with `--yes`. `apply` and `remove` refuse symlinked profile
directories and `user.js` files. Launching uses `std::process::Command`, never
a shell. CLI mistakes print a concise usage line and exit 2; operational failures print `bp: ...` and exit 1.
