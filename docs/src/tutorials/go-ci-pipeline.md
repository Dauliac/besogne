# CI pipeline with Go

This tutorial creates a multi-step CI pipeline: test, lint, coverage report, with proper DAG ordering.

## The manifest

```toml
name = "go-ci"
description = "Go CI pipeline: test + lint + coverage"

[inputs.go]
type = "binary"

[inputs.golangci-lint]
type = "binary"

[inputs.go-mod]
type = "file"
path = "go.mod"

[inputs.go-sum]
type = "file"
path = "go.sum"

[inputs.test]
type = "command"
phase = "exec"
run = ["go", "test", "-v", "-coverprofile=cover.out", "./..."]

[[inputs.test.ensure]]
type = "file"
path = "cover.out"
required = true

[inputs.lint]
type = "command"
phase = "exec"
run = ["golangci-lint", "run", "./..."]

[inputs.coverage]
type = "command"
phase = "exec"
run = ["go", "tool", "cover", "-html=cover.out", "-o", "coverage.html"]
after = ["test"]

[[inputs.coverage.ensure]]
type = "file"
path = "coverage.html"
required = true
```

## The DAG

```
test ──→ coverage
lint     (parallel with test, no dependency)
```

`test` and `lint` run in the same tier (parallel). `coverage` waits for `test` (it needs `cover.out`).

## Build and run

```bash
besogne run
```

Output:
```
go-ci — Go CI pipeline: test + lint + coverage
  checking 4 inputs...
  ✓ binary:go
  ✓ binary:golangci-lint
  ✓ file:go.mod
  ✓ file:go.sum

▶ test: go test -v -coverprofile=cover.out ./...
    === RUN   TestAdd
    --- PASS: TestAdd
  ✓ test  1.234s

▶ lint: golangci-lint run ./...
  ✓ lint  0.567s

▶ coverage: go tool cover -html=cover.out -o coverage.html
  ✓ coverage  0.123s

✅ 1.924s
```
