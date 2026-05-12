/*
 * besogne env tracker — LD_PRELOAD/DYLD_INSERT_LIBRARIES interposer.
 *
 * Wraps getenv() and secure_getenv() to log accessed env var names
 * to a file descriptor specified by BESOGNE_ENVTRACK_FD.
 *
 * Protocol: one var name per line (\n separated) on the tracking fd.
 * Duplicates are expected — the reader deduplicates.
 */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>

static int track_fd = -2; /* -2 = not initialized, -1 = disabled */

static int get_track_fd(void) {
    if (track_fd != -2) return track_fd;

    /* Bootstrap: call real getenv directly via dlsym to avoid recursion */
    char *(*real)(const char *) = dlsym(RTLD_NEXT, "getenv");
    if (!real) { track_fd = -1; return -1; }

    const char *fd_str = real("BESOGNE_ENVTRACK_FD");
    if (!fd_str || !*fd_str) { track_fd = -1; return -1; }

    track_fd = atoi(fd_str);
    if (track_fd < 0) track_fd = -1;
    return track_fd;
}

static void log_access(const char *name) {
    int fd = get_track_fd();
    if (fd < 0 || !name) return;

    /* Skip our own tracking var and common noise */
    if (strcmp(name, "BESOGNE_ENVTRACK_FD") == 0) return;
    if (strcmp(name, "LD_PRELOAD") == 0) return;
    if (strcmp(name, "DYLD_INSERT_LIBRARIES") == 0) return;

    size_t len = strlen(name);
    /* Write name + newline atomically (if < PIPE_BUF, which is >= 512) */
    char buf[512];
    if (len + 1 < sizeof(buf)) {
        memcpy(buf, name, len);
        buf[len] = '\n';
        write(fd, buf, len + 1);
    }
}

char *getenv(const char *name) {
    static char *(*real_getenv)(const char *) = NULL;
    if (!real_getenv) {
        real_getenv = dlsym(RTLD_NEXT, "getenv");
        if (!real_getenv) return NULL;
    }
    log_access(name);
    return real_getenv(name);
}

/* glibc extension — also intercept */
#ifdef __linux__
char *secure_getenv(const char *name) {
    static char *(*real_secure_getenv)(const char *) = NULL;
    if (!real_secure_getenv) {
        real_secure_getenv = dlsym(RTLD_NEXT, "secure_getenv");
        if (!real_secure_getenv) return getenv(name);
    }
    log_access(name);
    return real_secure_getenv(name);
}
#endif
