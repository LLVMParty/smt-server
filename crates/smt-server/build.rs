// Links Bitwuzla by fetching a prebuilt static archive at build time (like the
// `z3` crate's `gh-release` feature). FFI bindings are hand-written and checked
// in (`src/bitwuzla_bindings.rs`, pinned to 0.9.1), so this script only emits
// link directives.
//
// Linux/macOS/Windows-gnu fetch the upstream Bitwuzla release (GMP/MPFR via
// pkg-config). Windows-msvc fetches a self-contained MSVC archive (static
// GMP/MPFR, no DLLs) from the bitwuzla-msvc fork, since upstream ships only
// MinGW. Release layouts vary, so we search the extracted tree for
// `libbitwuzla.a`.

fn main() {
    if let Err(err) = bitwuzla::link() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

mod bitwuzla {
    use std::env;
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Bitwuzla release tag the upstream prebuilt archives are fetched from.
    /// Bump this and re-verify the constants in `src/bitwuzla_bindings.rs` (the
    /// `bitwuzla_backend` test is the drift canary). Keep `MSVC_TAG` in sync.
    const BITWUZLA_VERSION: &str = "0.9.1";
    const UPSTREAM_BASE: &str = "https://github.com/bitwuzla/bitwuzla/releases/download";

    /// The MSVC static archive is rebuilt from the 0.9.1 source under `cl.exe`
    /// and published on the bitwuzla-msvc fork's GitHub release, with GMP/MPFR baked in
    /// statically so the linked Rust binary has no extra DLL dependencies.
    const MSVC_TAG: &str = "bitwuzla-msvc-0.9.1";
    const MSVC_ASSET: &str = "Bitwuzla-Win64-x86_64-msvc-static";
    const MSVC_RELEASE_BASE: &str = "https://github.com/LLVMParty/bitwuzla-msvc/releases/download";

    /// How GMP/MPFR are resolved for a fetched archive.
    enum Flavor {
        /// Upstream prebuilt; GMP/MPFR resolved via pkg-config on the host.
        PkgConfig,
        /// Self-contained archive shipping statically-linked GMP/MPFR (MSVC).
        StaticShipped,
    }

    struct TargetSpec {
        /// Asset name (also the top-level dir inside the zip).
        asset: &'static str,
        /// GitHub release download base URL.
        release_base: &'static str,
        /// Release tag the asset lives under.
        tag: &'static str,
        flavor: Flavor,
    }

    /// Map the cargo target to the archive to fetch. `Err` for unsupported
    /// targets.
    fn target_spec() -> Result<TargetSpec, String> {
        let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let env_ = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
        let upstream = |asset: &'static str| TargetSpec {
            asset,
            release_base: UPSTREAM_BASE,
            tag: BITWUZLA_VERSION,
            flavor: Flavor::PkgConfig,
        };
        match (os.as_str(), arch.as_str(), env_.as_str()) {
            ("linux", "x86_64", _) => Ok(upstream("Bitwuzla-Linux-x86_64-static")),
            ("linux", "aarch64", _) => Ok(upstream("Bitwuzla-Linux-arm64-static")),
            ("macos", "aarch64", _) => Ok(upstream("Bitwuzla-macOS-arm64-static")),
            ("windows", "x86_64", "gnu") => Ok(upstream("Bitwuzla-Win64-x86_64-static")),
            ("windows", "x86_64", "msvc") => Ok(TargetSpec {
                asset: MSVC_ASSET,
                release_base: MSVC_RELEASE_BASE,
                tag: MSVC_TAG,
                flavor: Flavor::StaticShipped,
            }),
            _ => Err(format!(
                "the `bitwuzla` feature has no prebuilt Bitwuzla static archive for \
                 target `{os}-{arch}-{env_}`. Supported: x86_64/aarch64 Linux (gnu), \
                 aarch64 macOS, x86_64 Windows (gnu and msvc targets)."
            )),
        }
    }

    pub fn link() -> Result<(), String> {
        // vcpkg's `mpfr.lib` duplicates two GMP members (mp_clz_tab.obj,
        // version.obj) also in `gmp.lib`; MSVC link.exe warns LNK4255 on the
        // duplicate name. Both are read-only GMP data and only one is pulled,
        // so it's cosmetic -- silenced here (stripping at packaging would rebuild
        // each archive's symbol table for no gain).
        if env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "msvc" {
            println!("cargo:rustc-link-arg=/IGNORE:4255");
        }
        // Dev/testing escape hatch: link a locally-built Bitwuzla static tree
        // (e.g. an MSVC `libbitwuzla.a` built from source) instead of fetching
        // the release archive. The dir must hold the Bitwuzla archives; GMP/MPFR
        // are still resolved via pkg-config below.
        if let Ok(dir) = env::var("BITWUZLA_LIB_DIR") {
            return link_local(&PathBuf::from(dir));
        }
        let spec = target_spec()?;
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR not set")?);
        let cache_dir = out_dir.join("bitwuzla-prebuilt");
        std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create cache dir: {e}"))?;
        let zip_path = cache_dir.join(format!("{}.zip", spec.asset));
        let marker = cache_dir.join(format!("{}.ok", spec.asset));
        let extract_root = cache_dir.join(spec.asset);

        let lib_dir = match find_lib_dir(&extract_root) {
            Some(d) => d,
            None => {
                download(&spec, &zip_path)?;
                extract(&zip_path, &extract_root)?;
                match find_lib_dir(&extract_root) {
                    Some(d) => {
                        std::fs::write(&marker, b"ok").map_err(|e| format!("write marker: {e}"))?;
                        d
                    }
                    None => {
                        let mut msg = format!(
                            "extracted {}.zip but `libbitwuzla.a` was not found; extracted tree:\n",
                            spec.asset
                        );
                        list_tree(&extract_root, &mut msg);
                        return Err(msg);
                    }
                }
            }
        };

        match spec.flavor {
            Flavor::PkgConfig => link_pkgconfig(&lib_dir),
            Flavor::StaticShipped => link_static_shipped(&lib_dir),
        }?;
        println!("cargo:rerun-if-changed=build.rs");
        Ok(())
    }

    /// Upstream prebuilt: link the four Bitwuzla archives and resolve GMP/MPFR
    /// via pkg-config. Mirrors upstream `bitwuzla.pc`.
    fn link_pkgconfig(lib_dir: &Path) -> Result<(), String> {
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        // Dependency order: main archive before its sub-libraries, then
        // GMP/MPFR, then psapi (Windows only).
        for lib in ["bitwuzla", "bitwuzlals", "bitwuzlabv", "bitwuzlabb"] {
            println!("cargo:rustc-link-lib=static={lib}");
        }
        for (dep, ver) in [("gmp", "6.3"), ("mpfr", "4.2.1")] {
            pkg_config::Config::new()
                .atleast_version(ver)
                .probe(dep)
                .map_err(|e| {
                    format!(
                        "failed to locate {dep} >= {ver} via pkg-config: {e}\n\
                         Bitwuzla's static archive needs GMP and MPFR. Install them, e.g.:\n  \
                         apt install libgmp-dev libmpfr-dev  |  brew install gmp mpfr  |  \
                         MSYS2: pacman -S mingw-w64-x86_64-gmp mingw-w64-x86_64-mpfr"
                    )
                })?;
        }
        if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
            println!("cargo:rustc-link-lib=psapi");
        }
        Ok(())
    }

    /// MSVC self-contained archive: link the six Bitwuzla archives plus the
    /// statically-linked GMP/MPFR shipped inside the zip (no pkg-config,
    /// no DLL dependencies at runtime).
    fn link_static_shipped(lib_dir: &Path) -> Result<(), String> {
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        // Dependency order: Bitwuzla archives before their GMP/MPFR deps, then
        // psapi (Windows process/memory info used by cadical).
        for lib in [
            "bitwuzla",
            "bitwuzlals",
            "bitwuzlabv",
            "bitwuzlabb",
            "bzlarng",
            "bzlautil",
            "gmp",
            "mpfr",
        ] {
            println!("cargo:rustc-link-lib=static={lib}");
        }
        println!("cargo:rustc-link-lib=psapi");
        Ok(())
    }

    /// Link a locally-built Bitwuzla static tree (the `BITWUZLA_LIB_DIR` escape
    /// hatch). The directory must contain the Bitwuzla static archives produced
    /// by an upstream build (`libbitwuzla.a`, `libbitwuzlals.a`,
    /// `libbitwuzlabv.a`, `libbitwuzlabb.a`, `libbzlarng.a`, `libbzlautil.a`).
    /// GMP/MPFR are located via pkg-config, exactly as for the fetched archive.
    fn link_local(lib_dir: &Path) -> Result<(), String> {
        if !lib_dir.join("libbitwuzla.a").exists() {
            return Err(format!(
                "BITWUZLA_LIB_DIR={} does not contain `libbitwuzla.a`",
                lib_dir.display()
            ));
        }
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        // Upstream's from-source layout splits the archives across the root
        // and a `lib/` subdir (`libbitwuzla.a` + `libbzlautil.a` at the root,
        // the rest under `lib/`). Add both so all archives resolve.
        let support_dir = lib_dir.join("lib");
        if support_dir.is_dir() {
            println!("cargo:rustc-link-search=native={}", support_dir.display());
        }
        // Dependency order: the main archive before its sub-libraries, then the
        // RNG/util support libs, then GMP/MPFR, then psapi (Windows only).
        for lib in [
            "bitwuzla",
            "bitwuzlals",
            "bitwuzlabv",
            "bitwuzlabb",
            "bzlarng",
            "bzlautil",
        ] {
            println!("cargo:rustc-link-lib=static={lib}");
        }
        for (dep, ver) in [("gmp", "6.3"), ("mpfr", "4.2.1")] {
            pkg_config::Config::new()
                .atleast_version(ver)
                .probe(dep)
                .map_err(|e| {
                    format!(
                        "failed to locate {dep} >= {ver} via pkg-config: {e}\n\
                         Bitwuzla's static archive needs GMP and MPFR. Install them, e.g.:\n  \
                         apt install libgmp-dev libmpfr-dev  |  brew install gmp mpfr  |  \
                         vcpkg install gmp mpfr (Windows MSVC)"
                    )
                })?;
        }
        if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
            println!("cargo:rustc-link-lib=psapi");
        }
        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-env-changed=BITWUZLA_LIB_DIR");
        Ok(())
    }

    /// Recursively locate the directory containing `libbitwuzla.a` (handles the
    /// `lib/` and `lib/<multiarch>/` layouts the release packages use).
    fn find_lib_dir(root: &Path) -> Option<PathBuf> {
        if root.join("libbitwuzla.a").exists() {
            return Some(root.to_path_buf());
        }
        for entry in std::fs::read_dir(root).ok()?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(d) = find_lib_dir(&p) {
                    return Some(d);
                }
            }
        }
        None
    }

    fn download(spec: &TargetSpec, dest: &Path) -> Result<(), String> {
        let url = format!("{}/{}/{}.zip", spec.release_base, spec.tag, spec.asset);
        let mut cmd = Command::new("curl");
        cmd.arg("-fsSL")
            .arg("--retry")
            .arg("3")
            .arg("--retry-delay")
            .arg("2");
        // Authenticated read-only requests avoid 403s/rate-limits on CI runners.
        if let Ok(tok) = env::var("READ_ONLY_GITHUB_TOKEN") {
            if !tok.is_empty() {
                cmd.arg("-H").arg(format!("Authorization: Bearer {tok}"));
            }
        }
        cmd.arg("-o").arg(dest).arg(&url);
        let status = cmd.status().map_err(|e| {
            format!(
                "failed to run curl: {e}\n\
                 the `bitwuzla` feature downloads a prebuilt archive at build time; \
                 curl must be on PATH."
            )
        })?;
        if !status.success() {
            return Err(format!("curl exited {status} fetching {url}"));
        }
        Ok(())
    }

    fn extract(zip: &Path, dest: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dest).map_err(|e| format!("create extract dir: {e}"))?;
        // Build.rs runs on the host: `unzip` on Unix, `tar` on Windows (bsdtar,
        // shipped with Windows 10+, handles zip). Both exist in CI and dev shells.
        let status = if cfg!(windows) {
            Command::new("tar")
                .arg("-xf")
                .arg(zip)
                .arg("-C")
                .arg(dest)
                .status()
        } else {
            Command::new("unzip")
                .arg("-oq")
                .arg(zip)
                .arg("-d")
                .arg(dest)
                .status()
        }
        .map_err(|e| format!("failed to run extractor: {e}"))?;
        if !status.success() {
            return Err(format!(
                "extractor exited {status} extracting {}",
                zip.display()
            ));
        }
        Ok(())
    }

    fn list_tree(root: &Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let rel = p.strip_prefix(root).unwrap_or(&p).display();
            let _ = writeln!(out, "  {rel}");
            if p.is_dir() {
                list_tree(&p, out);
            }
        }
    }
}
