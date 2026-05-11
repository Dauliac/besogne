#!/bin/sh
# Level 3: leaf — arithmetic, loops, signal trap, parallel forks
set -e

echo "level3: start project=$PROJECT_NAME (pid=$$)"

# Trap to prove signal handling works
cleanup() { echo "level3: trap fired project=$PROJECT_NAME" >> $OUTPUT_PREFIX/level3.txt; }
trap cleanup EXIT

# Loop with arithmetic using FORK_COUNT
SUM=0
for i in $(seq 1 $FORK_COUNT); do
  SUM=$((SUM + i))
done
echo "level3: sum=$SUM fork_count=$FORK_COUNT" >> $OUTPUT_PREFIX/level3.txt

# Fork FORK_COUNT parallel children, wait for all
for i in $(seq 1 $FORK_COUNT); do
  (sleep $SLEEP_DELAY && echo "fork$i: done project=$PROJECT_NAME" >> $OUTPUT_PREFIX/forks.txt) &
done
wait

# Read from /dev/urandom to exercise I/O
RANDOM_HEX=$(head -c 8 /dev/urandom | od -A n -t x1 | tr -d ' \n')
echo "level3: random=$RANDOM_HEX" >> $OUTPUT_PREFIX/level3.txt

echo "level3: done"
