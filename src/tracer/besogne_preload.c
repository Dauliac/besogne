/*
 * besogne unified preload interposer — shared memory ring buffer.
 *
 * LD_PRELOAD (Linux) / DYLD_INSERT_LIBRARIES (macOS).
 *
 * Hooks: getenv, execve, fork, vfork, _exit, connect.
 * Events written to a shared memory ring buffer (~10ns per event, zero syscalls).
 *
 * Shared memory layout:
 *   [0..3]   atomic uint32_t write_pos
 *   [4..7]   uint32_t buf_size (total buffer bytes after header)
 *   [8..]    event data
 *
 * Event format:
 *   [tag:u8][pid:u32][payload_len:u16][payload:N bytes]
 *   Total header: 7 bytes + variable payload.
 *
 * Tags:
 *   'E' = getenv (payload = var name)
 *   'X' = execve (payload = binary path)
 *   'F' = fork   (payload = child_pid as u32 LE)
 *   'Q' = _exit  (payload = exit_code as i32 LE)
 *   'C' = connect (payload = AF:u16 + port:u16 + addr:4or16 bytes)
 *
 * BESOGNE_PRELOAD_FD: fd number of the shared memory mapping.
 * Parent creates the mapping before fork; child inherits it.
 */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <netinet/in.h>

/* ── Shared memory ring buffer ───────────────────────────────────── */

struct shm_header {
    volatile uint32_t write_pos;  /* atomic write cursor */
    uint32_t buf_size;            /* usable buffer size after header */
    /* buf[0..buf_size] follows */
};

static struct shm_header *shm = NULL;
static int initialized = 0;

static void init_shm(void) {
    if (initialized) return;
    initialized = 1;

    /* Get fd from env — use dlsym to avoid recursion */
    char *(*real_getenv)(const char *) = dlsym(RTLD_NEXT, "getenv");
    if (!real_getenv) return;

    const char *fd_str = real_getenv("BESOGNE_PRELOAD_FD");
    if (!fd_str || !*fd_str) return;

    int fd = 0;
    for (const char *p = fd_str; *p >= '0' && *p <= '9'; p++)
        fd = fd * 10 + (*p - '0');
    if (fd <= 0) return;

    /* mmap the shared memory fd (works on both Linux and macOS) */
    uint32_t header[2];
    if (pread(fd, header, 8, 0) != 8) return;
    uint32_t total_size = 8 + header[1];

    shm = (struct shm_header *)mmap(NULL, total_size,
        PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (shm == MAP_FAILED) { shm = NULL; return; }
}

/* Emit an event: atomic append to ring buffer. ~10ns, zero syscalls. */
static inline void emit(uint8_t tag, const void *payload, uint16_t len) {
    if (!shm) { init_shm(); if (!shm) return; }

    uint32_t total = 7 + len;
    uint32_t pos = __atomic_fetch_add(&shm->write_pos, total, __ATOMIC_RELAXED);
    if (pos + total > shm->buf_size) return;  /* buffer full — drop */

    char *p = ((char *)shm) + 8 + pos;
    *p++ = (char)tag;
    uint32_t pid = (uint32_t)getpid();
    memcpy(p, &pid, 4); p += 4;
    memcpy(p, &len, 2); p += 2;
    if (len > 0) memcpy(p, payload, len);
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

/* ── execve ──────────────────────────────────────────────────────── */

int execve(const char *path, char *const argv[], char *const envp[]) {
    static int (*real)(const char *, char *const[], char *const[]) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "execve");
    if (!real) return -1;

    if (path) {
        size_t len = strlen(path);
        if (len < 512) emit('X', path, (uint16_t)len);
    }

    return real(path, argv, envp);
}

/* Also hook execv — many programs use it */
int execv(const char *path, char *const argv[]) {
    static int (*real)(const char *, char *const[]) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "execv");
    if (!real) return -1;

    if (path) {
        size_t len = strlen(path);
        if (len < 512) emit('X', path, (uint16_t)len);
    }

    return real(path, argv);
}

/* ── fork ────────────────────────────────────────────────────────── */

pid_t fork(void) {
    static pid_t (*real)(void) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "fork");
    if (!real) return -1;

    pid_t child = real();
    if (child > 0) {
        /* Parent: log child PID */
        uint32_t cpid = (uint32_t)child;
        emit('F', &cpid, 4);
    } else if (child == 0) {
        /* Child: re-initialize shm pointer (inherited mapping still valid) */
        /* Nothing to do — mapping is inherited via MAP_SHARED */
    }
    return child;
}

pid_t vfork(void) {
    /* vfork is tricky — treat as fork for safety */
    return fork();
}

/* ── _exit ───────────────────────────────────────────────────────── */

void _exit(int status) {
    static void (*real)(int) __attribute__((noreturn)) = NULL;
    if (!real) real = dlsym(RTLD_NEXT, "_exit");

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
            /* IPv4: AF(2) + port(2) + addr(4) = 8 bytes */
            const struct sockaddr_in *in = (const struct sockaddr_in *)addr;
            uint8_t buf[8];
            uint16_t af = AF_INET;
            memcpy(buf, &af, 2);
            memcpy(buf + 2, &in->sin_port, 2);
            memcpy(buf + 4, &in->sin_addr, 4);
            emit('C', buf, 8);
        } else if (addr->sa_family == AF_INET6 && len >= sizeof(struct sockaddr_in6)) {
            /* IPv6: AF(2) + port(2) + addr(16) = 20 bytes */
            const struct sockaddr_in6 *in6 = (const struct sockaddr_in6 *)addr;
            uint8_t buf[20];
            uint16_t af = AF_INET6;
            memcpy(buf, &af, 2);
            memcpy(buf + 2, &in6->sin6_port, 2);
            memcpy(buf + 4, &in6->sin6_addr, 16);
            emit('C', buf, 20);
        }
        /* Skip AF_UNIX, AF_NETLINK, etc. — not interesting for dep tracking */
    }

    return real(fd, addr, len);
}
