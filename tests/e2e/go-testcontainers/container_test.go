package main

import (
	"context"
	"testing"

	"github.com/testcontainers/testcontainers-go"
	"github.com/testcontainers/testcontainers-go/wait"
)

// TestRedisContainer starts a Redis container using testcontainers-go,
// demonstrating that besogne can trace container subprocesses spawned
// by test frameworks in any language.
func TestRedisContainer(t *testing.T) {
	ctx := context.Background()

	req := testcontainers.ContainerRequest{
		Image:        "redis:7-alpine",
		ExposedPorts: []string{"6379/tcp"},
		WaitingFor:   wait.ForLog("Ready to accept connections"),
	}

	redis, err := testcontainers.GenericContainer(ctx, testcontainers.GenericContainerRequest{
		ContainerRequest: req,
		Started:          true,
	})
	if err != nil {
		t.Fatalf("failed to start redis: %v", err)
	}
	defer func() { _ = redis.Terminate(ctx) }()

	host, err := redis.Host(ctx)
	if err != nil {
		t.Fatalf("failed to get host: %v", err)
	}

	port, err := redis.MappedPort(ctx, "6379/tcp")
	if err != nil {
		t.Fatalf("failed to get port: %v", err)
	}

	t.Logf("redis running at %s:%s", host, port.Port())

	// Verify container is reachable
	state, err := redis.State(ctx)
	if err != nil {
		t.Fatalf("failed to get state: %v", err)
	}
	if !state.Running {
		t.Fatal("redis container should be running")
	}
}

// TestNginxContainer starts an nginx container to show multi-container tests.
func TestNginxContainer(t *testing.T) {
	ctx := context.Background()

	req := testcontainers.ContainerRequest{
		Image:        "nginx:alpine",
		ExposedPorts: []string{"80/tcp"},
		WaitingFor:   wait.ForHTTP("/"),
	}

	nginx, err := testcontainers.GenericContainer(ctx, testcontainers.GenericContainerRequest{
		ContainerRequest: req,
		Started:          true,
	})
	if err != nil {
		t.Fatalf("failed to start nginx: %v", err)
	}
	defer func() { _ = nginx.Terminate(ctx) }()

	host, err := nginx.Host(ctx)
	if err != nil {
		t.Fatalf("failed to get host: %v", err)
	}

	port, err := nginx.MappedPort(ctx, "80/tcp")
	if err != nil {
		t.Fatalf("failed to get port: %v", err)
	}

	t.Logf("nginx running at %s:%s", host, port.Port())
}
