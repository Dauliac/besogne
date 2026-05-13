/*
 * besogne unified preload interposer — shared memory ring buffer.
 *
 * LD_PRELOAD (Linux) / DYLD_INSERT_LIBRARIES (macOS).
 * ~10ns per event (atomic fetch_add + memcpy, zero syscalls).
 *
 * Shared memory layout:
 *   [0..3]   atomic uint32_t write_pos
 *   [4..7]   uint32_t buf_size (total buffer bytes after header)
 *   [8..]    event data
 *
 * Event format:
 *   [tag:u8][pid:u32][payload_len:u16][payload:N bytes]
 *
 * Tags:
 *   'E' = getenv      (payload = var name)
 *   'X' = execve      (payload = binary path)
 *   'F' = fork        (payload = child_pid as u32 LE)
 *   'Q' = _exit       (payload = exit_code as i32 LE)
 *   'C' = connect     (payload = AF:u16 + port:u16 + addr:4or16)
 *   'O' = openat      (payload = flags:u8 + path)  flags: 0=read, 1=write, 2=rw
 *   'D' = getaddrinfo (payload = hostname)
 *   'U' = unlink      (payload = path)
 *   'R' = rename      (payload = old_len:u16 + old + new)
 *   'L' = dlopen      (payload = library path)
 *   'N' = net_io      (payload = rx_bytes:u64 LE + tx_bytes:u64 LE) — emitted on _exit
 */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdarg.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netdb.h>

/* ── Shared memory ring buffer ───────────────────────────────────── */

struct shm_header {
    volatile uint32_t write_pos;
    uint32_t buf_size;
};

static struct shm_header *shm = NULL;
static int initialized = 0;

static void init_shm(void) {
    if (initialized) return;
    initialized = 1;

    char *(*real_getenv)(const char *) = dlsym(RTLD_NEXT, "getenv");
    if (!real_getenv) return;

    const char *fd_str = real_getenv("BESOGNE_PRELOAD_FD");
    if (!fd_str || !*fd_str) return;

    int fd = 0;
    for (const char *p = fd_str; *p >= '0' && *p <= '9'; p++)
        fd = fd * 10 + (*p - '0');
    if (fd <= 0) return;

    uint32_t header[2];
    if (pread(fd, header, 8, 0) != 8) return;
    uint32_t total_size = 8 + header[1];

    shm = (struct shm_header *)mmap(NULL, total_size,
        PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (shm == MAP_FAILED) { shm = NULL; return; }
}

static inline void emit(uint8_t tag, const void *payload, uint16_t len) {
    if (!shm) { init_shm(); if (!shm) return; }

    uint32_t total = 7 + len;
    uint32_t pos = __atomic_fetch_add(&shm->write_pos, total, __ATOMIC_RELAXED);
    if (pos + total > shm->buf_size) return;

    char *p = ((char *)shm) + 8 + pos;
    *p++ = (char)tag;
    uint32_t pid = (uint32_t)getpid();
    memcpy(p, &pid, 4); p += 4;
    memcpy(p, &len, 2); p += 2;
    if (len > 0) memcpy(p, payload, len);
}

/* Helper: emit a string event (tag + path) with length check */
static inline void emit_path(uint8_t tag, const char *path) {
    if (!path) return;
    size_t len = strlen(path);
    if (len > 0 && len < 512) emit(tag, path, (uint16_t)len);
}

/* Helper: skip noise paths for file tracking */
static inline int is_noise_path(const char *path) {
    if (!path) return 1;
    /* Skip virtual/system filesystems */
    if (path[0] == '/' && (
        strncmp(path, "/dev/", 5) == 0 ||
        strncmp(path, "/proc/", 6) == 0 ||
        strncmp(path, "/sys/", 5) == 0 ||
        strncmp(path, "/tmp/", 5) == 0 ||
        strncmp(path, "/var/tmp/", 9) == 0 ||
        strncmp(path, "/run/", 5) == 0
    )) return 1;
    /* Skip pipes and sockets (no path or special) */
    if (path[0] == '\0') return 1;
    return 0;
}

/* ── getenv ──────────────────────────────────────────────────────── */

char *getenv(const char *name) {
    static char *(*real)(const char *) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "getenv");
    if (!real) return NULL;

    if (name && name[0] != '_' &&
        strcmp(name, "BESOGNE_PRELOAD_FD") != 0 &&
        strcmp(name, "LD_PRELOAD") != 0 &&
        strcmp(name, "DYLD_INSERT_LIBRARIES") != 0)
    {
        size_t len = strlen(name);
        if (len < 256) emit('E', name, (uint16_t)len);
    }

    return real(name);
}

#ifdef __linux__
char *secure_getenv(const char *name) {
    static char *(*real)(const char *) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "secure_getenv");
    if (!real) return getenv(name);

    if (name && name[0] != '_') {
        size_t len = strlen(name);
        if (len < 256) emit('E', name, (uint16_t)len);
    }
    return real(name);
}
#endif

/* ── execve / execv ──────────────────────────────────────────────── */

int execve(const char *path, char *const argv[], char *const envp[]) {
    static int (*real)(const char *, char *const[], char *const[]) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "execve");
    if (!real) return -1;

    emit_path('X', path);
    return real(path, argv, envp);
}

int execv(const char *path, char *const argv[]) {
    static int (*real)(const char *, char *const[]) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "execv");
    if (!real) return -1;

    emit_path('X', path);
    return real(path, argv);
}

/* ── fork / vfork ────────────────────────────────────────────────── */

pid_t fork(void) {
    static pid_t (*real)(void) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "fork");
    if (!real) return -1;

    pid_t child = real();
    if (child > 0) {
        uint32_t cpid = (uint32_t)child;
        emit('F', &cpid, 4);
    }
    return child;
}

pid_t vfork(void) {
    return fork();
}

/* ── Per-process network I/O counters ─────────────────────────────── */
/* Only count on internet sockets (AF_INET/AF_INET6), not Unix domain. */
/* Track which fds are internet sockets via a bitset.                   */

static volatile uint64_t net_rx_total = 0;
static volatile uint64_t net_tx_total = 0;

/* Bitset: fd N is an internet socket if bit N is set. Supports fd 0..1023. */
#define MAX_TRACKED_FD 1024
static volatile uint64_t inet_fd_bits[MAX_TRACKED_FD / 64] = {0};

static inline void mark_inet_fd(int fd) {
    if (fd >= 0 && fd < MAX_TRACKED_FD)
        __atomic_fetch_or(&inet_fd_bits[fd / 64], 1ULL << (fd % 64), __ATOMIC_RELAXED);
}
static inline void clear_inet_fd(int fd) {
    if (fd >= 0 && fd < MAX_TRACKED_FD)
        __atomic_fetch_and(&inet_fd_bits[fd / 64], ~(1ULL << (fd % 64)), __ATOMIC_RELAXED);
}
static inline int is_inet_fd(int fd) {
    if (fd < 0 || fd >= MAX_TRACKED_FD) return 0;
    return (__atomic_load_n(&inet_fd_bits[fd / 64], __ATOMIC_RELAXED) >> (fd % 64)) & 1;
}

/* Hook socket() to track which fds are internet sockets */
int socket(int domain, int type, int protocol) {
    static int (*real)(int, int, int) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "socket");
    if (!real) return -1;

    int fd = real(domain, type, protocol);
    if (fd >= 0 && (domain == AF_INET || domain == AF_INET6)) {
        mark_inet_fd(fd);
    }
    return fd;
}

/* Hook close() to clear the bitset */
int close(int fd) {
    static int (*real)(int) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "close");
    if (!real) return -1;

    clear_inet_fd(fd);
    return real(fd);
}

/* ── _exit ───────────────────────────────────────────────────────── */

void _exit(int status) {
    static void (*real)(int) __attribute__((noreturn)) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "_exit");

    /* Emit network I/O totals before exit */
    uint64_t rx = __atomic_load_n(&net_rx_total, __ATOMIC_RELAXED);
    uint64_t tx = __atomic_load_n(&net_tx_total, __ATOMIC_RELAXED);
    if (rx > 0 || tx > 0) {
        uint8_t buf[16];
        memcpy(buf, &rx, 8);
        memcpy(buf + 8, &tx, 8);
        emit('N', buf, 16);
    }

    int32_t code = (int32_t)status;
    emit('Q', &code, 4);

    real(status);
    __builtin_unreachable();
}

/* ── connect ─────────────────────────────────────────────────────── */

int connect(int fd, const struct sockaddr *addr, socklen_t len) {
    static int (*real)(int, const struct sockaddr *, socklen_t) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "connect");
    if (!real) return -1;

    if (addr) {
        if (addr->sa_family == AF_INET && len >= sizeof(struct sockaddr_in)) {
            const struct sockaddr_in *in = (const struct sockaddr_in *)addr;
            uint8_t buf[8];
            uint16_t af = AF_INET;
            memcpy(buf, &af, 2);
            memcpy(buf + 2, &in->sin_port, 2);
            memcpy(buf + 4, &in->sin_addr, 4);
            emit('C', buf, 8);
        } else if (addr->sa_family == AF_INET6 && len >= sizeof(struct sockaddr_in6)) {
            const struct sockaddr_in6 *in6 = (const struct sockaddr_in6 *)addr;
            uint8_t buf[20];
            uint16_t af = AF_INET6;
            memcpy(buf, &af, 2);
            memcpy(buf + 2, &in6->sin6_port, 2);
            memcpy(buf + 4, &in6->sin6_addr, 16);
            emit('C', buf, 20);
        }
    }

    return real(fd, addr, len);
}

/* ── openat / open — file access tracking ────────────────────────── */

#ifdef __linux__
int openat(int dirfd, const char *path, int flags, ...) {
    static int (*real)(int, const char *, int, ...) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "openat");
    if (!real) return -1;

    if (path && !is_noise_path(path)) {
        /* Encode: flags_byte(1) + path */
        size_t plen = strlen(path);
        if (plen < 510) {
            uint8_t buf[512];
            uint8_t fmode = 0; /* 0=read, 1=write, 2=rw */
            int acc = flags & O_ACCMODE;
            if (acc == O_WRONLY) fmode = 1;
            else if (acc == O_RDWR) fmode = 2;
            if (flags & (O_CREAT | O_TRUNC | O_APPEND)) fmode |= 1;
            buf[0] = fmode;
            memcpy(buf + 1, path, plen);
            emit('O', buf, (uint16_t)(1 + plen));
        }
    }

    /* Forward varargs for mode parameter */
    va_list ap;
    va_start(ap, flags);
    mode_t mode = 0;
    if (flags & O_CREAT) mode = va_arg(ap, mode_t);
    va_end(ap);
    return real(dirfd, path, flags, mode);
}
#endif

int open(const char *path, int flags, ...) {
    static int (*real)(const char *, int, ...) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "open");
    if (!real) return -1;

    if (path && !is_noise_path(path)) {
        size_t plen = strlen(path);
        if (plen < 510) {
            uint8_t buf[512];
            uint8_t fmode = 0;
            int acc = flags & O_ACCMODE;
            if (acc == O_WRONLY) fmode = 1;
            else if (acc == O_RDWR) fmode = 2;
            if (flags & (O_CREAT | O_TRUNC | O_APPEND)) fmode |= 1;
            buf[0] = fmode;
            memcpy(buf + 1, path, plen);
            emit('O', buf, (uint16_t)(1 + plen));
        }
    }

    va_list ap;
    va_start(ap, flags);
    mode_t mode = 0;
    if (flags & O_CREAT) mode = va_arg(ap, mode_t);
    va_end(ap);
    return real(path, flags, mode);
}

/* ── getaddrinfo — DNS resolution tracking ───────────────────────── */

int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints, struct addrinfo **res) {
    static int (*real)(const char *, const char *,
                       const struct addrinfo *, struct addrinfo **) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "getaddrinfo");
    if (!real) return -1;

    if (node) {
        emit_path('D', node);
    }

    return real(node, service, hints, res);
}

/* ── unlink — file deletion (side effect) ────────────────────────── */

int unlink(const char *path) {
    static int (*real)(const char *) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "unlink");
    if (!real) return -1;

    if (path && !is_noise_path(path)) {
        emit_path('U', path);
    }

    return real(path);
}

#ifdef __linux__
int unlinkat(int dirfd, const char *path, int flags) {
    static int (*real)(int, const char *, int) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "unlinkat");
    if (!real) return -1;

    if (path && !is_noise_path(path)) {
        emit_path('U', path);
    }

    return real(dirfd, path, flags);
}
#endif

/* ── rename — file mutation (side effect) ────────────────────────── */

int rename(const char *oldpath, const char *newpath) {
    static int (*real)(const char *, const char *) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "rename");
    if (!real) return -1;

    if (oldpath && newpath && !is_noise_path(oldpath)) {
        /* Encode: old_len:u16 + old + new */
        size_t olen = strlen(oldpath);
        size_t nlen = strlen(newpath);
        if (olen + nlen + 2 < 510) {
            uint8_t buf[512];
            uint16_t ol = (uint16_t)olen;
            memcpy(buf, &ol, 2);
            memcpy(buf + 2, oldpath, olen);
            memcpy(buf + 2 + olen, newpath, nlen);
            emit('R', buf, (uint16_t)(2 + olen + nlen));
        }
    }

    return real(oldpath, newpath);
}

/* ── dlopen — dynamic library loading ────────────────────────────── */

void *dlopen(const char *path, int flags) {
    static void *(*real)(const char *, int) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "dlopen");
    if (!real) return NULL;

    if (path) {
        emit_path('L', path);
    }

    return real(path, flags);
}

/* ── send / recv — per-process network byte counting ─────────────── */
/* Only hook socket-specific calls (not read/write which also hit files). */
/* ~5ns overhead per call: just an atomic add on the counter.            */

ssize_t send(int fd, const void *buf, size_t len, int flags) {
    static ssize_t (*real)(int, const void *, size_t, int) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "send");
    if (!real) return -1;

    ssize_t n = real(fd, buf, len, flags);
    if (n > 0 && is_inet_fd(fd))
        __atomic_fetch_add(&net_tx_total, (uint64_t)n, __ATOMIC_RELAXED);
    return n;
}

ssize_t sendto(int fd, const void *buf, size_t len, int flags,
               const struct sockaddr *addr, socklen_t addrlen) {
    static ssize_t (*real)(int, const void *, size_t, int,
                           const struct sockaddr *, socklen_t) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "sendto");
    if (!real) return -1;

    ssize_t n = real(fd, buf, len, flags, addr, addrlen);
    if (n > 0 && is_inet_fd(fd))
        __atomic_fetch_add(&net_tx_total, (uint64_t)n, __ATOMIC_RELAXED);
    return n;
}

ssize_t recv(int fd, void *buf, size_t len, int flags) {
    static ssize_t (*real)(int, void *, size_t, int) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "recv");
    if (!real) return -1;

    ssize_t n = real(fd, buf, len, flags);
    if (n > 0 && is_inet_fd(fd))
        __atomic_fetch_add(&net_rx_total, (uint64_t)n, __ATOMIC_RELAXED);
    return n;
}

ssize_t recvfrom(int fd, void *buf, size_t len, int flags,
                 struct sockaddr *addr, socklen_t *addrlen) {
    static ssize_t (*real)(int, void *, size_t, int,
                           struct sockaddr *, socklen_t *) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "recvfrom");
    if (!real) return -1;

    ssize_t n = real(fd, buf, len, flags, addr, addrlen);
    if (n > 0 && is_inet_fd(fd))
        __atomic_fetch_add(&net_rx_total, (uint64_t)n, __ATOMIC_RELAXED);
    return n;
}
