@echo off
setlocal enableextensions enabledelayedexpansion
rem ============================================================================
rem  Reproducible build of the self-contained MSVC static Bitwuzla archive that
rem  crates/smt-server/build.rs fetches for the x86_64-pc-windows-msvc target.
rem
rem  Output: %WORK_DIR%\Bitwuzla-Win64-x86_64-msvc-static.zip
rem    Publish it to https://github.com/LLVMParty/smt-server/releases under tag
rem    bitwuzla-msvc-0.9.1  (asset name must be exactly
rem    Bitwuzla-Win64-x86_64-msvc-static.zip -- matches MSVC_ASSET in build.rs).
rem
rem  Prereqs (on PATH, or located automatically):
rem    * Visual Studio 2022 (cl.exe 19.4x; vcvars64.bat is searched for under
rem      C:\Program Files\Microsoft Visual Studio\2022\<edition>\). VS 2022 is
rem      required: the shim is validated on cl 19.44, and VS 18's cl 19.5x
rem      declares __builtin_fmaf as a native intrinsic that conflicts with the
rem      shim's polyfill. Override the path with the VCVARS env var if needed.
rem    * meson (>=1.4), ninja, git, python3
rem    * vcpkg  (pass its root as %1, default F:\Repos\vcpkg)
rem
rem  Usage:
rem    build.bat [VCPKG_DIR] [WORK_DIR]
rem      VCPKG_DIR  default F:\Repos\vcpkg
rem      WORK_DIR    default %~dp0work   (clones bitwuzla here, builds here)
rem
rem  NOTE: the vcpkg root is held in %VCPKG% (not VCPKG_ROOT) because
rem  vcvars64.bat overwrites VCPKG_ROOT. --vcpkg-root forces it on every call.
rem ============================================================================
set "VCPKG=%~1"
if "%VCPKG%"=="" set "VCPKG=F:\Repos\vcpkg"
set "WORK_DIR=%~2"
if "%WORK_DIR%"=="" set "WORK_DIR=%~dp0work"
set "SHIM_DIR=%~dp0shim"
set "PATCH=%~dp0bitwuzla-0.9.1-msvc.patch"
set "BITWUZLA_TAG=0.9.1"
set "TRIPLET=x64-windows-static-md"
set "ASSET=Bitwuzla-Win64-x86_64-msvc-static"

if not exist "%VCPKG%\vcpkg.exe" (echo ERROR: vcpkg.exe not found at !VCPKG! & exit /b 1)
where meson >nul 2>&1 || (echo ERROR: meson not on PATH & exit /b 1)
where ninja >nul 2>&1 || (echo ERROR: ninja not on PATH & exit /b 1)
where git >nul 2>&1 || (echo ERROR: git not on PATH & exit /b 1)

rem --- locate VS 2022's vcvars64.bat (try standard editions; VCVARS overrides) ---
if not defined VCVARS goto :find_vcvars
call "%VCVARS%" >nul
if errorlevel 1 (echo ERROR: vcvars failed & exit /b 1)
goto :vcvars_done
:find_vcvars
set "VCVARS="
for %%E in (Enterprise Professional Community BuildTools) do if exist "C:\Program Files\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=C:\Program Files\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat"
if not defined VCVARS (echo ERROR: VS 2022 vcvars64.bat not found. Set the VCVARS env var to its path. & exit /b 1)
call "%VCVARS%" >nul
if errorlevel 1 (echo ERROR: vcvars failed & exit /b 1)
:vcvars_done

rem --- vcpkg: static-md GMP/MPFR (static lib, dynamic CRT -> matches Rust /MD,
rem     no CRT mismatch, no GMP/MPFR DLL deps) + pkgconf host tool ---
echo === vcpkg: gmp mpfr (%TRIPLET%) + pkgconf ===
"%VCPKG%\vcpkg.exe" --vcpkg-root "%VCPKG%" install gmp mpfr --triplet %TRIPLET% || exit /b 1
"%VCPKG%\vcpkg.exe" --vcpkg-root "%VCPKG%" install pkgconf --triplet x64-windows || exit /b 1
set "VCPKG_INST=%VCPKG%\installed\%TRIPLET%"
set "VCPKG_TOOLS=%VCPKG%\installed\x64-windows\tools\pkgconf"
if not exist "%VCPKG_TOOLS%\pkg-config.exe" copy /y "%VCPKG_TOOLS%\pkgconf.exe" "%VCPKG_TOOLS%\pkg-config.exe" >nul
set "PKG_CONFIG_PATH=%VCPKG_INST%\lib\pkgconfig"
set "PKG_CONFIG=%VCPKG_TOOLS%\pkg-config.exe"
set "PATH=%VCPKG_TOOLS%;%PATH%"

rem --- clone bitwuzla 0.9.1 + apply patch ---
if not exist "%WORK_DIR%\bitwuzla-src\.git" (
  echo === clone bitwuzla %BITWUZLA_TAG% ===
  git clone --depth 1 --branch %BITWUZLA_TAG% https://github.com/bitwuzla/bitwuzla "%WORK_DIR%\bitwuzla-src" || exit /b 1
)
pushd "%WORK_DIR%\bitwuzla-src"
git checkout -- . 2>nul
echo === apply patch ===
git apply --check "%PATCH%" 2>nul && (git apply "%PATCH%" || (popd & exit /b 1)) || echo patch already applied
popd

rem --- meson setup + ninja (the 6 static archives, skip the CLI) ---
set "SHIMFLAGS=/D__WIN32 /I%SHIM_DIR% /FImsvc_compat.h"
set "CFLAGS=%SHIMFLAGS%"
set "CXXFLAGS=%SHIMFLAGS%"
pushd "%WORK_DIR%\bitwuzla-src"
if exist build_msvc rmdir /s /q build_msvc
echo === meson setup ===
meson setup build_msvc --default-library=static -Dpython=false -Dtesting=disabled -Dunit_testing=disabled || (popd & exit /b 1)
echo === ninja (6 archives) ===
ninja -C build_msvc src/libbitwuzla.a src/libbzlautil.a src/lib/libbitwuzlals.a src/lib/libbitwuzlabv.a src/lib/libbitwuzlabb.a src/lib/libbzlarng.a || (popd & exit /b 1)
popd

rem --- assemble the self-contained zip ---
set "STAGE=%WORK_DIR%\%ASSET%"
if exist "%STAGE%" rmdir /s /q "%STAGE%"
mkdir "%STAGE%\lib"
copy /y "%WORK_DIR%\bitwuzla-src\build_msvc\src\libbitwuzla.a"        "%STAGE%\lib\" >nul
copy /y "%WORK_DIR%\bitwuzla-src\build_msvc\src\libbzlautil.a"        "%STAGE%\lib\" >nul
copy /y "%WORK_DIR%\bitwuzla-src\build_msvc\src\lib\libbitwuzlals.a"  "%STAGE%\lib\" >nul
copy /y "%WORK_DIR%\bitwuzla-src\build_msvc\src\lib\libbitwuzlabv.a"  "%STAGE%\lib\" >nul
copy /y "%WORK_DIR%\bitwuzla-src\build_msvc\src\lib\libbitwuzlabb.a"  "%STAGE%\lib\" >nul
copy /y "%WORK_DIR%\bitwuzla-src\build_msvc\src\lib\libbzlarng.a"     "%STAGE%\lib\" >nul
copy /y "%VCPKG_INST%\lib\gmp.lib"                                    "%STAGE%\lib\" >nul
copy /y "%VCPKG_INST%\lib\mpfr.lib"                                   "%STAGE%\lib\" >nul
set "ZIP=%WORK_DIR%\%ASSET%.zip"
if exist "%ZIP%" del "%ZIP%"
pushd "%WORK_DIR%"
tar -caf "%ASSET%.zip" "%ASSET%"
popd
echo === DONE: %ZIP% ===
dir "%ZIP%"
