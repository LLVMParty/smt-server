# Bitwuzla MSVC static build

Reproducible recipe that builds the **self-contained MSVC static Bitwuzla
archive** consumed by `crates/smt-server/build.rs` for the
`x86_64-pc-windows-msvc` target, and publishes it as a GitHub release asset.

Upstream Bitwuzla ships only a MinGW (GCC) Windows build, whose GNU object
archive MSVC's linker cannot consume. This package rebuilds Bitwuzla 0.9.1 from
source under `cl.exe`, with GMP/MPFR **statically linked in** (vcpkg
`x64-windows-static-md` triplet: static lib + dynamic CRT), so the linked Rust
binary has no GMP/MPFR DLL dependencies and no CRT mismatch.

## Contents

| File | Purpose |
|------|---------|
| `build.bat` | One-shot driver: vcpkg install → clone 0.9.1 → patch → meson → ninja → zip. |
| `bitwuzla-0.9.1-msvc.patch` | The source patches required for `cl.exe` (unified diff vs tag `0.9.1`). |
| `shim/msvc_compat.h` | GCC-ism polyfills (`__attribute__`, `__builtin_*`), force-included (`/FI`). |
| `shim/unistd.h` | Minimal POSIX `<unistd.h>` → MSVC CRT shim. |

## Prerequisites

- Visual Studio 2022 (`cl.exe`; `vcvars64.bat` is located automatically via
  `vswhere`).
- `meson` (>=1.4), `ninja`, `git`, `python3` on `PATH`.
- vcpkg (pass its root as the first argument; defaults to `F:\Repos\vcpkg`).

## Build

```bat
build.bat [VCPKG_DIR] [WORK_DIR]
```

- `VCPKG_DIR` — vcpkg root (default `F:\Repos\vcpkg`).
- `WORK_DIR` — where bitwuzla is cloned & built (default `.\work`).

Output: `<WORK_DIR>\Bitwuzla-Win64-x86_64-msvc-static.zip` — a zip whose
top-level dir `Bitwuzla-Win64-x86_64-msvc-static/lib/` holds the six Bitwuzla
archives plus `gmp.lib`/`mpfr.lib`.

## Publish

The asset name and release tag **must** match `build.rs` (`MSVC_ASSET` /
`MSVC_TAG`):

```sh
gh release create bitwuzla-msvc-0.9.1 \
  --repo LLVMParty/smt-server \
  --title "Bitwuzla MSVC static 0.9.1" \
  --notes "Self-contained MSVC static Bitwuzla 0.9.1 (GMP/MPFR statically linked)." \
  <WORK_DIR>/Bitwuzla-Win64-x86_64-msvc-static.zip
```

`build.rs` then fetches
`https://github.com/LLVMParty/smt-server/releases/download/bitwuzla-msvc-0.9.1/Bitwuzla-Win64-x86_64-msvc-static.zip`
for the `msvc` target.

## Recipe summary

1. `vcpkg install gmp mpfr --triplet x64-windows-static-md` (static lib, dynamic
   CRT → `__GMP_LIBGMP_DLL=0` in `gmp.h`, so objects emit direct `__gmpz_*` /
   `mpfr_*` symbols linkable into a static archive — no `__imp_` import stubs).
2. Clone `bitwuzla` tag `0.9.1`; `git apply bitwuzla-0.9.1-msvc.patch`.
3. `meson setup --default-library=static` with
   `CFLAGS=CXXFLAGS=/D__WIN32 /I<shim> /FImsvc_compat.h` (activates cadical's
   Windows code paths and injects the GCC-ism polyfills) and
   `PKG_CONFIG_PATH` pointing at the static-md triplet.
4. `ninja` the six archives.
5. Stage the six `.a` + `gmp.lib` + `mpfr.lib` into `lib/` and zip.

The three source patches: `src/util/integer.h` adds `#include <string>`;
`src/main/time_limit.cpp` adds `#include <chrono>` and guards the glibc<2.34
pthread condvar workaround behind `#ifndef _MSC_VER`.

## Known MSVC test-suite limitations (unused by the backend)

The full upstream regression suite (`meson test -C build … -Dtesting=enabled`,
4200 tests) passes **~4150–4160 / 4200** on this VS 2022 (`cl` 19.44) build.
The exact failure count is **non-deterministic** (observed 37–49 distinct
failing tests across runs): the underlying defect is heap memory corruption
whose manifestation depends on allocation layout. Every failure falls in one
of two code paths that `BitwuzlaBackend` never exercises:

- **Interpolant generation** (`get-interpolant_*`, ~36–39 tests) —
  non-deterministic memory corruption. The same input either completes cleanly
  (~190 B of valid output) or crashes with `0xC0000005` (access violation),
  first emitting 83 B–628 KB of varying non-UTF-8 garbage to stdout. Triggered
  by `(set-option :produce-interpolants true)` + `(get-interpolant …)`.
  Root-causing the exact UAF/OOB needs an AddressSanitizer build; deferred.
- **Letified SMT-LIB printer** (`printer_nested_letify`, 1–2 test invocations)
  — two failure modes. (1) The same non-deterministic `0xC0000005` crash +
  garbage as above, in `smt2_printer.cpp`'s `letify()` traversal. (2) When it
  does *not* crash, a **deterministic** logic bug: the letify traversal drops
  the repeated 2nd occurrence of a subterm, emitting malformed output with no
  `let` bindings — e.g. `(bvmul X)` with one argument instead of
  `(bvmul X X)`, vs. the expected `(let ((_let1 …)) (bvmul _let1 _let1))`.

Every path the backend uses **passes**: `solver_*`, `solver_bv_*`,
`get-model_*`, `get-value_*`, `get-unsat-core_*`, `get-unsat-assumptions_*`,
`rewrite_*`, `preprocess_*`, fp, array. `BitwuzlaBackend` does QF_BV solve /
model extraction / named unsat cores / bit-hunt optimization via the C API — it
never sets `PRODUCE_INTERPOLANTS`, calls no `get-interpolant*` symbol, and its
only string output is `bitwuzla_term_value_get_str_fmt` (a value-constant
formatter that bypasses the SMT-LIB printer) — so these failures do not affect
it.
