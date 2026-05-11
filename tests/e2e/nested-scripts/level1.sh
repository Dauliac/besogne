#!/bin/sh
# Level 1: orchestrator — calls level2, spawns a background job, writes results
set -e

echo "level1: start (pid=$$)"

# Spawn a background process that writes after a short delay
(sleep 0.1 && echo "background: done (pid=$$)" >> results/background.txt) &
BG_PID=$!

# Call level 2
sh ./level2.sh

# Write own output
echo "level1: wrote" >> results/level1.txt

# Wait for background job
wait $BG_PID
echo "level1: background finished"

# Fork a subshell pipeline
echo "hello-from-pipe" | tr 'a-z' 'A-Z' | tee results/pipe.txt > /dev/null

# Run a Docker hello-world container
docker run --rm hello-world > results/docker.txt 2>&1

echo "level1: done"
