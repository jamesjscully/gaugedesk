{
  # ─────────────────────────────────────────────────────────────────────────
  # REPRODUCIBLE-BUILD GROUNDWORK — Tier 2 attestation measurement input.
  #
  # This flake builds the gaugewright backend workspace *deterministically* so the
  # SHA-256 of the resulting artifact can serve as the expected `CodeMeasurement`
  # an attesting host must reproduce (ADR 0040 / ADR 0042). It is the build half
  # of the `MeasurementStore` seam (crates/app/src/measurement_store.rs,
  # ATTEST-10): a verifier only trusts a host's measurement if that measurement
  # ties back to a *known, reproducible* build — this is how that build is made
  # known. See DEFERRED.md `D-ATTEST` item 5 (reproducible-build CI/CD pipeline,
  # ATTEST-15) and specs/implementation/test-and-release-infra.md Tier 2
  # "Reproducible builds".
  #
  # NEEDS-INFRA NOTE: the flake evaluates and `nix build` runs on any Nix host
  # with flakes enabled. Producing the *attestation* measurement (the SHA-256 of
  # a container/VM image the host runs and a TEE then measures) additionally
  # needs the release pipeline that wraps this artifact into the deployable image
  # and publishes the digest into the `MeasurementStore` — that pipeline runs in
  # CI/release, not in the loopback scaffold. Nothing here is on the `cargo test`
  # path (the backend gate is `cargo test --manifest-path Cargo.toml`); this file
  # is consumed only by `nix`.
  #
  # Determinism contract:
  #   * nixpkgs is pinned to an exact revision (flake.lock pins the narHash too),
  #     so the toolchain (rustc/cargo, the C toolchain, coreutils) is fixed.
  #   * the Cargo dependency graph is pinned by ./Cargo.lock; rustPlatform vendors
  #     it offline (no network during the build → no drift).
  #   * SOURCE_DATE_EPOCH + a fixed build are set so timestamps do not leak in.
  #   * the Tauri desktop shell (src-tauri/) is a separate workspace needing system
  #     webkit and is intentionally NOT built here, matching Cargo.toml's exclude.
  # ─────────────────────────────────────────────────────────────────────────
  description = "gaugewright backend workspace — reproducible build for the attestation measurement (ATTEST-10 / ATTEST-15)";

  inputs = {
    # Pinned to an exact revision for determinism; flake.lock additionally pins
    # the narHash. Bump deliberately (and re-publish the measurement) — never
    # float, or the measurement is not reproducible.
    nixpkgs.url = "github:NixOS/nixpkgs/cbb5cf358f50aa6acc9efd6113b7bcfbc352cd73";
  };

  outputs = { self, nixpkgs }:
    let
      # The platforms a reproducible artifact is built for. The attestation host
      # runs Linux; the other systems are for local `nix build` on dev machines.
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
      # The version lives in the root [workspace.package] (member crates inherit
      # it via `version.workspace = true`).
      workspaceToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      workspaceVersion = workspaceToml.workspace.package.version or "0.0.0";
    in
    {
      # `nix build` (and `nix build .#gaugewright-backend`) produces the workspace
      # release binaries. `nix build .#measurement` additionally writes the
      # SHA-256 of the primary artifact — the value published to the
      # MeasurementStore.
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };

          gaugewright-backend = pkgs.rustPlatform.buildRustPackage {
            pname = "gaugewright-backend";
            version = workspaceVersion;

            # Only the inputs that affect the build, so the derivation hash is a
            # function of the actual sources (not editor state / target dirs).
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter = path: type:
                let rel = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
                in
                  rel == "Cargo.toml"
                  || rel == "Cargo.lock"
                  || rel == "crates" || pkgs.lib.hasPrefix "crates/" rel
                  # specs/ is read by the build (proptests reference the models).
                  || rel == "specs" || pkgs.lib.hasPrefix "specs/" rel;
            };

            # Vendor the exact dependency graph offline from the committed lock.
            cargoLock.lockFile = ./Cargo.lock;

            # Build the backend workspace only; the Tauri shell is excluded in
            # Cargo.toml and needs system webkit not present here.
            cargoBuildFlags = [ "--workspace" ];
            # The artifact is the build product; tests run in the cargo gate / CI,
            # not in the reproducible-build derivation (keeps the measurement a
            # pure function of source, not of nondeterministic test ordering).
            doCheck = false;

            # Reproducibility knobs: fix timestamps and locale so nothing
            # environment-derived leaks into the bytes.
            SOURCE_DATE_EPOCH = "1";
            env.LC_ALL = "C";

            meta = {
              description = "gaugewright backend workspace (reproducible artifact for attestation)";
              # Surfaced so a release pipeline can assert it built the right thing.
              longDescription = "Deterministic build of the gaugewright Cargo workspace; its SHA-256 is the CodeMeasurement published to the MeasurementStore (ATTEST-10).";
            };
          };

          # The published measurement: the SHA-256 of the built artifact tree.
          # This is the exact value a release pipeline registers as a
          # MeasurementRecord (BuildId → CodeMeasurement) in the MeasurementStore.
          # `nix build .#measurement` → result/sha256.txt + result/measurement.txt.
          measurement = pkgs.runCommand "gaugewright-measurement"
            { nativeBuildInputs = [ pkgs.coreutils ]; }
            ''
              mkdir -p "$out"
              # Hash the artifact tree deterministically: sort by path, hash each
              # file, then hash the manifest of (sha256, path) pairs. Stable across
              # rebuilds because the input store path is content-addressed.
              ( cd ${gaugewright-backend} && find . -type f -print0 | sort -z \
                  | xargs -0 sha256sum ) > "$out/manifest.txt"
              digest=$(sha256sum "$out/manifest.txt" | cut -d' ' -f1)
              printf '%s\n' "$digest" > "$out/sha256.txt"
              # The CodeMeasurement shape consumed by MeasurementStore::register
              # (BuildId.image_ref / .version → CodeMeasurement(sha256)).
              printf 'image_ref=gaugewright-backend\nversion=${workspaceVersion}\nmeasurement=%s\n' \
                "$digest" > "$out/measurement.txt"
            '';
        in
        {
          inherit gaugewright-backend measurement;
          default = gaugewright-backend;
        });
    };
}
