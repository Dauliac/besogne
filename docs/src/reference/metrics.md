# Metrics and tracing

besogne instruments every command execution with process-level metrics.

## Essential mode (default)

Near-zero overhead. Uses `wait4` + `rusage` for the top-level command.

| Metric | Source | Always available |
|---|---|---|
| Wall time | `Instant::now` delta | Yes |
| Exit code | `wait4` | Yes |
| CPU user/sys | `wait4` → `rusage` | Linux |
| Peak RSS | `wait4` → `rusage.ru_maxrss` | Linux |
| Voluntary ctx switches | `wait4` → `rusage.ru_nvcsw` | Linux |
| Involuntary ctx switches | `wait4` → `rusage.ru_nivcsw` | Linux |

## Future: subprocess tracing

Planned via netlink proc connector (`CAP_NET_ADMIN`) with ptrace fallback:

- Process tree (fork/exec/exit for every subprocess)
- Per-subprocess CPU, memory, I/O
- Container detection (docker run/podman)
- Network bytes via `/proc/{pid}/net/dev`

## Future: deep mode

Filtered syscall-level interception via ptrace:

- `--deep=files` — openat, read, write per file
- `--deep=memory` — mmap, brk, munmap
- `--deep=network` — socket, connect, per-socket bytes
- `--deep=signals` — signal handlers, delivery
- `--deep=scheduling` — futex, nanosleep
- `--deep=errors` — all syscalls returning -1

## Metrics in JSON output

All metrics appear in `command_end` events:

```json
{
  "event": "command_end",
  "name": "test",
  "exit_code": 0,
  "wall_ms": 1234,
  "user_ms": 800,
  "sys_ms": 200,
  "max_rss_kb": 45000
}
```
