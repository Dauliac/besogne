# Output modes

besogne supports three output modes, selectable via `--log-format` (`-l`).

## Human (default)

Interactive, compact. Only what matters.

```
hello v0.0.1 — My first besogne
  checking 2 inputs...
  ✓ binary:go
  ✓ file:go.mod

▶ test: go test -v ./...
    === RUN   TestAdd
    --- PASS: TestAdd
  ✓ test  1.234s  0.80u/0.20s  45MB

✅ 1.250s
```

- Subprocess tree hidden on success
- Stderr shown only on failure
- Secrets masked
- Timing and memory for significant commands

## CI

Non-interactive, structured, GitHub Actions annotations.

```
::group::hello v0.0.1 — My first besogne
  checking 2 inputs...
  [PASS] binary:go
  [PASS] file:go.mod
::endgroup::
::group::test: go test -v ./...
  [PASS] test  1.234s
::endgroup::
::notice::PASS 1.250s
```

## JSON (NDJSON)

Machine-readable, one JSON object per line.

```json
{"event":"start","name":"hello","version":"0.0.1"}
{"event":"seal_start","input_count":2}
{"event":"probe","input":"binary:go","success":true}
{"event":"probe","input":"file:go.mod","success":true}
{"event":"seal_end"}
{"event":"command_start","name":"test","exec":["go","test","./..."]}
{"event":"command_end","name":"test","exit_code":0,"wall_ms":1234,"user_ms":800,"sys_ms":200,"max_rss_kb":45000}
{"event":"summary","exit_code":0,"wall_ms":1250}
```

Query with jq:

```bash
./my-task --log-format json 2>&1 | jq 'select(.event == "command_end")'
```
