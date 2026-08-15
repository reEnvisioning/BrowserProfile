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
bp create work --browser firefox
bp list
bp apply work --browser firefox
bp default get --browser firefox
bp default set work --browser firefox
bp launch work --browser firefox
bp remove work --browser firefox
```

## Safety

Unmanaged-profile and browser-default mutations require lowercase `y` confirmation.
