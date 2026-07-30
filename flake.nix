{
  description = "Slopium language toolchain: slopic compiler and slopium project manager";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          toolchain = pkgs.rustPlatform.buildRustPackage {
            pname = "slopium";
            version = "0.3.3";
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter = path: type:
                let name = baseNameOf path;
                in name != ".git" && name != ".notes" && name != "target";
            };

            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper pkgs.stdenv.cc ];

            cargoBuildFlags = [ "--workspace" ];
            cargoTestFlags = [ "--workspace" ];

            installPhase = ''
              runHook preInstall
              mkdir -p "$out/bin"
              cp \
                target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release/slopic \
                target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release/slopium \
                target/${pkgs.stdenv.hostPlatform.rust.rustcTarget}/release/slopium-lsp \
                "$out/bin/"
              runHook postInstall
            '';

            postInstall = ''
              wrapProgram "$out/bin/slopic" \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.stdenv.cc ]}
              wrapProgram "$out/bin/slopium" \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.stdenv.cc ]}
            '';

            meta = {
              description = "Native S-expression compiler and project manager";
              license = pkgs.lib.licenses.mit;
              platforms = [ "x86_64-linux" ];
              mainProgram = "slopium";
            };
          };
        in
        {
          default = toolchain;
          slopium = toolchain;
        });

      apps = forAllSystems (system:
        let
          package = self.packages.${system}.default;
        in
        {
          default = {
            type = "app";
            program = "${package}/bin/slopium";
            meta.description = "Run the Slopium project manager";
          };
          slopium = {
            type = "app";
            program = "${package}/bin/slopium";
            meta.description = "Run the Slopium project manager";
          };
          slopic = {
            type = "app";
            program = "${package}/bin/slopic";
            meta.description = "Run the low-level Slopium compiler";
          };
          slopium-lsp = {
            type = "app";
            program = "${package}/bin/slopium-lsp";
            meta.description = "Run the Slopium language server";
          };
        });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustc
              cargo
              rustfmt
              clippy
              stdenv.cc
              binutils
              gdb
              valgrind
            ];

            shellHook = ''
              echo "Slopium development shell"
              echo "  cargo build --workspace"
              echo "  cargo test --workspace"
            '';
          };
        });

      formatter = forAllSystems (system:
        let pkgs = import nixpkgs { inherit system; };
        in pkgs.nixfmt);

      checks = forAllSystems (system: {
        package = self.packages.${system}.default;
      });
    };
}
