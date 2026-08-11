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
`~/.local/share/browserprofile`). Browser `profiles.ini` files are read from
`$XDG_CONFIG_HOME` (or `~/.config`). Names are deliberately restricted to
letters, digits, `_`, `-`, and `.` and cannot begin with `.`.

`remove` only deletes profiles carrying `.browserprofile-owned`. `apply` and
`remove` refuse symlinked profile directories and `user.js` files. Launching
uses `std::process::Command`, never a shell.
