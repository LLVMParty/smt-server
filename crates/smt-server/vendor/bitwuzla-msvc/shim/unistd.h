#pragma once
/*
 * Minimal POSIX <unistd.h> shim for building cadical + bitwuzla with MSVC.
 * Only symbols referenced by the *library* (not the CLI/mobical) are mapped to
 * the MSVC CRT. Compiled into every C++ TU via /I<shim> + /FImsvc_compat.h.
 */
#include <io.h>        /* _isatty, _access, _read/_write/_close, _open */
#include <process.h>   /* _getpid, _exit */
#include <direct.h>    /* _getcwd, _chdir, _rmdir */
#include <stdio.h>     /* _popen, _pclose */
#include <sys/stat.h>  /* struct stat, stat() */
#include <sys/types.h>

typedef intptr_t ssize_t;
typedef int pid_t;

#ifndef STDIN_FILENO
#define STDIN_FILENO 0
#endif
#ifndef STDOUT_FILENO
#define STDOUT_FILENO 1
#endif
#ifndef STDERR_FILENO
#define STDERR_FILENO 2
#endif

/* access() mode flags (POSIX; MSVC <io.h> does not define R_OK/W_OK). */
#ifndef F_OK
#define F_OK 0
#endif
#ifndef R_OK
#define R_OK 4
#endif
#ifndef W_OK
#define W_OK 2
#endif
#ifndef X_OK
#define X_OK 0 /* Windows access() ignores the execute bit */
#endif

/* stat() type-check macros (POSIX; MSVC <sys/stat.h> lacks S_ISDIR/S_ISFIFO). */
#ifndef S_IFMT
#define S_IFMT 0xF000
#endif
#ifndef S_IFDIR
#define S_IFDIR 0x4000
#endif
#ifndef S_IFIFO
#define S_IFIFO 0x1000
#endif
#ifndef S_ISDIR
#define S_ISDIR(m) (((m) & S_IFMT) == S_IFDIR)
#endif
#ifndef S_ISFIFO
#define S_ISFIFO(m) (((m) & S_IFMT) == S_IFIFO)
#endif

/* POSIX name -> MSVC CRT underscore equivalent. */
#define isatty _isatty
#define getpid _getpid
#define access _access
#define unlink _unlink
#define rmdir _rmdir
#define getcwd _getcwd
#define chdir _chdir
#define close _close
#define read _read
#define write _write
#define popen _popen
#define pclose _pclose
