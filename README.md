# BrowserProfile

`bp` manages Firefox and LibreWolf profiles from reviewed templates.

## Install

```sh
cargo install --git https://github.com/reEnvisioning/BrowserProfile.git
nix run github:reEnvisioning/BrowserProfile -- list
```

Cargo and Nix install both `bp` and `browserprofile`.

```nix
inputs.browserprofile.url = "github:reEnvisioning/BrowserProfile";
packages.${pkgs.system}.default = inputs.browserprofile.packages.${pkgs.system}.default;
```

## Use

```sh
bp create --name work --browser firefox --default no
bp list
bp apply @1
bp default get --browser firefox
bp default set @1
bp launch @1
bp remove @1 --yes
```

## Profile IDs

Profiles have a global canonical ID such as `@1`; `#` IDs are rejected. Names are scoped to a browser, so name-based commands require `--browser` or a controlling terminal for a browser prompt. `bp create [@ID]` prompts for omitted name, browser, and default choice; pressing Enter at the browser prompt selects Firefox. `--name`, `--browser`, and `--default yes|no` avoid those prompts. Bare `bp launch` lists every safe launchable profile, optionally filtered with `--browser`, then selects an `@ID` from `/dev/tty`.

`$XDG_DATA_HOME/browserprofile/profiles.catalog` is the strictly validated, atomically written global ID catalog. Existing safe registered profiles get a stable ID when selected or listed. Output is `@id\tbrowser\tname`.

## Safety

Prompts use `/dev/tty` and stderr, never piped stdin. Unmanaged-profile and browser-default mutations require lowercase `y` confirmation. Browser launches use literal process arguments; this document does not authorize a live launch.
