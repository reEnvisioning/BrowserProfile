# BrowserProfile

`bp` is a small standalone CLI for safe runtime Firefox and LibreWolf profile management.

```sh
bp create work --browser firefox --default
bp apply default --browser firefox --backup
bp list --browser firefox
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

`remove` only deletes profiles carrying `.browserprofile-owned`. Removing a
browser-default profile always asks `Remove browser default profile NAME? [y|N]`,
even with `--yes`. `apply` and `remove` refuse symlinked profile directories and
`user.js` files. Launching uses `std::process::Command`, never a shell.
