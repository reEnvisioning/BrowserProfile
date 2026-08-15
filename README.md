# BrowserProfile

`bp` safely manages Firefox and LibreWolf profiles at runtime. The package also
installs `browserprofile` as an alias.

## Install

```sh
nix run github:reEnvisioning/BrowserProfile -- list
```

## Use

```sh
bp create work --browser firefox --default
bp apply default --browser firefox --backup
bp list                         # both browsers, qualified
bp default get --browser firefox
bp launch work --browser firefox -- https://example.org
bp remove work --browser firefox --yes
```

## Paths and safety

- Owned profiles: `${XDG_DATA_HOME:-$HOME/.local/share}/browserprofile/`
- Firefox registry: `${XDG_CONFIG_HOME:-$HOME/.config}/mozilla/firefox/profiles.ini`
- LibreWolf registry: `$HOME/.librewolf/profiles.ini`
- Names: letters, digits, `_`, `-`, `.`; no leading `.`

Mutations preserve unrelated registry sections. Unmanaged targets and removal
of a browser default require explicit lowercase `y`; `--yes` cannot bypass
those prompts. Symlinked profile directories and `user.js` files are refused.
Browsers are launched directly, never through a shell. The first LibreWolf
command may import BrowserProfile-owned entries from its obsolete nested
registry; the old file is left untouched.

## Compatibility

Linux CLI; independent of compositor, desktop environment, and display
protocol. Firefox or LibreWolf must already be installed for `launch`.
