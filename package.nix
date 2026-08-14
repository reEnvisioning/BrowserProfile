{ lib, rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "browserprofile";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  postInstall = ''
    ln -s bp "$out/bin/browserprofile"
  '';
  meta = {
    description = "Safe runtime management for Firefox-family profiles";
    license = lib.licenses.mit;
    mainProgram = "bp";
  };
}
