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

## LD_PRELOAD interposer

besogne includes a shared memory LD_PRELOAD interposer (`besogne_preload.c`) for fast process telemetry with near-zero overhead (~10ns per event, zero syscalls). It uses `mmap(MAP_SHARED)` for a lock-free ring buffer.

The interposer tracks:
- `getenv()` calls — which env vars are actually accessed
- `execve()` calls — which binaries are executed
- `fork()` / `exit()` — process tree
- `connect()` — network connections
- `open()` / `unlink()` / `rename()` — file I/O
- `getaddrinfo()` — DNS resolution
- `dlopen()` — dynamic library loading

This data feeds into undeclared dependency detection and process tree visualization.

## Process tree

Every command's subprocess tree is captured:

| Field | Source |
|---|---|
| PID / PPID | `fork` / `wait4` |
| Command line | `/proc/{pid}/cmdline` |
| Exit code | `wait4` |
| CPU user/sys per process | `wait4` → `rusage` |
| Peak RSS per process | `wait4` → `rusage.ru_maxrss` |
| Disk I/O bytes | `/proc/{pid}/io` |

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
  "max_rss_kb": 45000,
  "disk_read_bytes": 1048576,
  "disk_write_bytes": 524288,
  "processes_spawned": 3
}
```
