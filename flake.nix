{
  description = "Snap - development environment";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.*.tar.gz";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
    nix-github-actions = {
      url = "github:nix-community/nix-github-actions";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, crane, advisory-db, nix-github-actions }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forEachSupportedSystem = f: nixpkgs.lib.genAttrs supportedSystems (system: f {
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default self.overlays.default ];
        };
        inherit system;
      });
    in
    {
      overlays.default = final: prev: {
        rustToolchain = final.rust-bin.fromRustupToolchainFile ./rust/rust-toolchain.toml;
      };

      packages = forEachSupportedSystem ({ pkgs, ... }:
        let
          craneLib = (crane.mkLib pkgs).overrideToolchain pkgs.rustToolchain;
          src = craneLib.cleanCargoSource ./rust;
          commonArgs = { inherit src; };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          bin = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            doCheck = false;
          });
        in
        { default = bin; }
      );

      checks = forEachSupportedSystem ({ pkgs, ... }:
        let
          craneLib = (crane.mkLib pkgs).overrideToolchain pkgs.rustToolchain;
          src = craneLib.cleanCargoSource ./rust;
          commonArgs = { inherit src; };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          fmt = craneLib.cargoFmt { inherit src; };

          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets --all-features -- -D warnings";
          });

          test = craneLib.cargoTest (commonArgs // {
            inherit cargoArtifacts;
            cargoTestExtraArgs = "--all-features";
          });

          deny = craneLib.cargoDeny { inherit src; };

          audit = craneLib.cargoAudit {
            inherit src;
            inherit advisory-db;
          };

          doc = craneLib.cargoDoc (commonArgs // {
            inherit cargoArtifacts;
            RUSTDOCFLAGS = "-D warnings";
            cargoDocExtraArgs = "--no-deps --all-features";
          });

          coverage = craneLib.cargoLlvmCov (commonArgs // {
            inherit cargoArtifacts;
            cargoLlvmCovExtraArgs = "--workspace --all-features";
            installPhase = "touch $out";
          });
        }
      );

      githubActions = nix-github-actions.lib.mkGithubMatrix {
        checks = nixpkgs.lib.getAttrs [ "x86_64-linux" ] self.checks;
      };

      apps = forEachSupportedSystem ({ pkgs, ... }:
        let
          nightlyToolchain = pkgs.rust-bin.selectLatestNightlyWith (toolchain:
            toolchain.default.override {
              extensions = [ "miri" "rust-src" ];
            }
          );
        in {
        miri = {
          type = "app";
          program = toString (pkgs.writeShellScript "miri" ''
            set -euo pipefail
            export PATH="${nightlyToolchain}/bin:$PATH"
            cd rust
            cargo miri test
          '');
        };

        validate = {
          type = "app";
          program = toString (pkgs.writeShellScript "validate" ''
            exec nix develop --command validate
          '');
        };

        validate-all = {
          type = "app";
          program = toString (pkgs.writeShellScript "validate-all" ''
            exec nix develop --command validate-all
          '');
        };
      });

      devShells = forEachSupportedSystem ({ pkgs, ... }:
        let
          acceptance = pkgs.writeShellScriptBin "acceptance" ''
            set -euo pipefail
            cd "$(git rev-parse --show-toplevel)"
            bin="$(nix build .#default --no-link --print-out-paths)/bin/snap"
            cd test-harness
            npm install --silent 2>&1
            npx tsx src/cli.ts --candidate "$bin"
          '';
          validate = pkgs.writeShellScriptBin "validate" ''
            set -euo pipefail
            cd "$(git rev-parse --show-toplevel)"
            echo "==> Running Nix flake checks..."
            nix flake check --keep-going
            echo "==> Running Miri..."
            nix run .#miri
            echo "==> Running acceptance tests..."
            acceptance
            echo "==> All validations passed!"
          '';
          validate-all = pkgs.writeShellScriptBin "validate-all" ''
            set -euo pipefail
            cd "$(git rev-parse --show-toplevel)"
            validate
            echo "==> Running mutation testing (diff vs main)..."
            git diff origin/main...HEAD > /tmp/mutants-diff.patch
            if [ -s /tmp/mutants-diff.patch ]; then
              cd rust
              cargo mutants --in-diff /tmp/mutants-diff.patch --in-place -vV --timeout 300
            else
              echo "    No diff vs main, skipping"
            fi
            echo "==> All validations passed!"
          '';
          cargo-mutants-diff = pkgs.writeShellScriptBin "cargo-mutants-diff" ''
            set -euo pipefail
            cd "$(git rev-parse --show-toplevel)"
            git diff origin/main...HEAD > /tmp/mutants-diff.patch
            if [ -s /tmp/mutants-diff.patch ]; then
              cd rust
              cargo mutants --in-diff /tmp/mutants-diff.patch --in-place -vV --timeout 300
            else
              echo "No diff vs main, nothing to test"
            fi
          '';
        in {
        default = pkgs.mkShell {
          packages = with pkgs; [
            rustToolchain
            cargo-audit
            cargo-deny
            cargo-edit
            cargo-llvm-cov
            cargo-mutants
            cargo-nextest
            cargo-watch
            rust-analyzer
            nodejs
            acceptance
            validate
            validate-all
            cargo-mutants-diff
          ];

          env = {
            RUST_SRC_PATH = "${pkgs.rustToolchain}/lib/rustlib/src/rust/library";
          };
        };
      });
    };
}
