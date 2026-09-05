# BrowserProfile

`bp` creates, applies, lists, selects, removes, and launches Firefox and LibreWolf profiles from reviewed `user.js` templates.

## Install

```sh
cargo install --git https://github.com/reEnvisioning/BrowserProfile.git
nix run github:reEnvisioning/BrowserProfile -- list
```

Cargo and Nix install both `bp` and `browserprofile`.

## Use

```sh
bp create --name work --browser firefox --default no
bp list
bp apply @1
bp default set @1
bp launch @1
bp remove @1 --yes
```

Profiles have global `@ID`s and browser-scoped names; name-based commands need `--browser` or a terminal browser choice. `bp list` shows BrowserProfile-created profiles; `bp list --all` also shows profiles registered by the browsers. `bp create` uses the embedded template by default. Pass `--template FILE` to `create` or `apply` to copy a regular template file unchanged; without `--name`, `search.user.js` becomes the profile name `search` (other files use their stem).

## Behavior and safety

BrowserProfile reads and updates Firefox's `$XDG_CONFIG_HOME/mozilla/firefox/profiles.ini` and LibreWolf's `$HOME/.librewolf/profiles.ini`, so profiles remain visible to their browsers. It marks profiles it creates; mutating an unmanaged profile requires lowercase `y` confirmation, and removal is destructive. It rejects traversal and symlinked paths, writes files atomically, and launches browsers with literal process arguments rather than a shell.
