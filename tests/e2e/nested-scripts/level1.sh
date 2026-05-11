#!/bin/sh
# Level 1: orchestrator — calls level2, spawns a background job, writes results
set -e

echo "level1: start project=$PROJECT_NAME (pid=$$)"

# Spawn a background process that writes after a short delay
(sleep $SLEEP_DELAY && echo "background: done project=$PROJECT_NAME (pid=$$)" >> $OUTPUT_PREFIX/background.txt) &
BG_PID=$!

# Call level 2
sh ./level2.sh

# Write own output
echo "level1: project=$PROJECT_NAME wrote" >> $OUTPUT_PREFIX/level1.txt

# Wait for background job
wait $BG_PID
echo "level1: background finished"

# Fork a subshell pipeline
echo "hello-from-$PROJECT_NAME" | tr 'a-z' 'A-Z' | tee $OUTPUT_PREFIX/pipe.txt > /dev/null

# Run a Docker hello-world container
docker run --rm hello-world > $OUTPUT_PREFIX/docker.txt 2>&1

echo "level1: done"
