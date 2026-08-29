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
bp create --browser firefox --default no
cp templates/template.user.js search.user.js
bp create --template search.user.js --browser firefox --default no
bp list
bp apply @1 --template search.user.js
bp default get --browser firefox
bp default set @1
bp launch @1
bp remove @1 --yes
```

## Profile IDs

Profiles have a global canonical ID such as `@1`; `#` IDs are rejected. Names are scoped to a browser, so name-based commands require `--browser` or a controlling terminal for a browser prompt. `bp create [@ID]` uses the embedded `template.user.js` unless `--template FILE` selects an external file. Without `--name`, the profile name comes from that template filename: `search.user.js` becomes `search`; other files use their ordinary stem. `--name` wins. `--browser` and `--default yes|no` avoid their prompts; pressing Enter at the browser prompt selects Firefox. Bare `bp launch` lists every safe launchable profile, optionally filtered with `--browser`, then selects an `@ID` from `/dev/tty`.

`$XDG_DATA_HOME/browserprofile/profiles.catalog` is the strictly validated, atomically written global ID catalog. Existing safe registered profiles get a stable ID when selected or listed. Output is `@id\tbrowser\tname`.

## Templates and safety

Firefox and LibreWolf use the same embedded `templates/template.user.js` bytes. BrowserProfile authors general UI settings, including top/horizontal tabs and current tab and search/address-bar controls, plus basic user-facing privacy and security settings; `user.js` reapplies them at startup, and browser UI or policies can override them. Default search-engine selection remains browser database state; the template otherwise excludes handlers, per-site permissions, OS integration, and rollout, migration, branding, endpoint, and lock internals.

`--template FILE` for `create` or `apply` accepts only a regular file up to 1 MiB and copies its bytes unchanged; it does not interpret browser preferences. Copy `templates/template.user.js`, edit that copy, then pass it with `--template FILE`.

Prompts use `/dev/tty` and stderr, never piped stdin. Unmanaged-profile and browser-default mutations require lowercase `y` confirmation. Browser launches use literal process arguments; this document does not authorize a live launch.
