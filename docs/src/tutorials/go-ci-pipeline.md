# CI pipeline with Go

This tutorial creates a multi-step CI pipeline: test, lint, coverage report, with proper DAG ordering.

## The manifest

```toml
name = "go-ci"
description = "Go CI pipeline: test + lint + coverage"

[nodes.go]
type = "binary"

[nodes.golangci-lint]
type = "binary"

[nodes.go-mod]
type = "file"
path = "go.mod"

[nodes.go-sum]
type = "file"
path = "go.sum"

[nodes.test]
type = "command"
phase = "exec"
run = ["go", "test", "-v", "-coverprofile=cover.out", "./..."]

[nodes.cover-out]
type = "file"
path = "cover.out"
parents = ["test"]

[nodes.lint]
type = "command"
phase = "exec"
run = ["golangci-lint", "run", "./..."]

[nodes.coverage]
type = "command"
phase = "exec"
run = ["go", "tool", "cover", "-html=cover.out", "-o", "coverage.html"]
parents = ["cover-out"]

[nodes.coverage-html]
type = "file"
path = "coverage.html"
parents = ["coverage"]
```

## The DAG

```
test ──→ cover-out ──→ coverage ──→ coverage-html
lint     (parallel with test, no dependency)
```

`test` and `lint` run in the same tier (parallel). `coverage` waits for `cover-out` — a file postcondition of `test`. This gives finer-grained caching: if `cover.out` hasn't changed, `coverage` can skip.

## Build and run

```bash
besogne run
```

Output:
```
go-ci — Go CI pipeline: test + lint + coverage
  checking 4 seals...
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
