{
  description = "Nix Local Cache - A local build service and cache for Nix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    
    # For building Rust (highly recommended over raw rustPlatform for workspaces)
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    
    # Rust toolchain overlay (needed for crane sometimes, or just good practice)
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        # --- Rust Backend Build (using crane) ---
        craneLib = crane.mkLib pkgs;
        
        # Common arguments for all crate builds
        commonArgs = {
          src = pkgs.lib.cleanSourceWith {
            src = craneLib.path ./.;
            filter = path: type:
              (craneLib.filterCargoSources path type) || (builtins.match ".*/\\.sqlx.*" path != null);
          };
          strictDeps = true;
          
          buildInputs = [
            pkgs.openssl
            pkgs.pkg-config
            pkgs.sqlite
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
            pkgs.darwin.apple_sdk.frameworks.Security
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
          
          nativeBuildInputs = [
            pkgs.pkg-config
          ];
          
          # Use sqlx-data.json for offline builds
          SQLX_OFFLINE = "true";
        };

        # Build artifacts (dependencies) only once
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Server Binary
        nix-local-cache-server = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "-p nix-local-cache-server";
          # Ensure we can find migrations at runtime if needed, 
          # though sqlx usually embeds them if using the macro. 
          # Our code uses sqlx::migrate!() which embeds them.
        });

        # Client Binary
        nix-local-cache-client = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoExtraArgs = "-p nix-local-cache-client";
        });

        # --- Frontend Build ---
        # We use buildNpmPackage, but since you use Bun, we can try mkBunDerivation 
        # if available or just standard npm hooks (bun is compatible with package-lock if generated).
        # For stability in Nix, it's often safer to stick to npm/pnpm lockfiles. 
        # I see you have `bun.lock`. Nixpkgs has limited direct Bun support.
        # Simplest stable way: use `buildNpmPackage` but we need `package-lock.json`.
        # Alternatively, allow bun to run in a derivation with fixed output hash (impure-ish but works).
        
        # Let's stick to a simple shell derivation using bun if we can't easily get package-lock.
        # Actually, let's try to generate a package-lock.json on the fly? No.
        
        # Better approach: Use `pkgs.buildNpmPackage` by generating a lock file locally once.
        # But to avoid forcing you to do that, I will create a derivation that uses bun 
        # with a pre-fetched cache.
        
        # SIMPLIFICATION FOR NOW:
        # I will define a script that builds the frontend using `bun` available in the environment.
        # For strictly pure Nix builds, we'd need a proper lockfile.
        
        # Let's assume for now we provide a "shell" to build it manually or
        # use a derivation that allows network (sandbox = false) which is bad.
        
        # Standard way:
        # 1. `cd frontend && bun install && bun run build`
        # We need `bun.lockb`. Nix doesn't parse binary locks well.
        
        # Decision: I will export a simple derivation that runs `bun build` but 
        # it might fail in pure mode without network. 
        # For this iteration, I'll leave frontend packaging as a TODO/Manual step 
        # or provide a "dev" build script.
        
        # WAIT! You have `package.json`. I can use `importNpmLock` if I had a lockfile.
        # I will skip strictly packaging the frontend asset *inside* Nix for this specific turn 
        # because of the Bun lockfile complexity, unless I generate a `package-lock.json`.
        # I will define the server and client packages first.
        
      in
      {
        packages = {
          default = nix-local-cache-server;
          server = nix-local-cache-server;
          client = nix-local-cache-client;
        };

        # Development shell (keeps your existing devenv mostly, or replaces it)
        # I'll keep devenv.nix separate for your dev flow.
      }
    ) // {
      # Non-system-specific outputs (Modules)
      nixosModules = {
        server = import ./modules/server.nix;
        default = self.nixosModules.server;
      };
    };
}
