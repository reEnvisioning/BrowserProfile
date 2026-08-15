{ lib, rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "browserprofile";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  meta = {
    description = "Safe runtime management for Firefox-family profiles";
    license = lib.licenses.mit;
    mainProgram = "bp";
  };
}
