#!/bin/sh
set -e
echo "level3: start (pid=$$)"
cleanup() { echo "level3: trap fired" >> results/level3.txt; }
trap cleanup EXIT
SUM=0
for i in 1 2 3 4 5; do SUM=$((SUM + i)); done
echo "level3: sum=$SUM" >> results/level3.txt
for i in 1 2 3; do
  (sleep 0.02 && echo "fork$i: done" >> results/forks.txt) &
done
wait
RANDOM_HEX=$(head -c 8 /dev/urandom | od -A n -t x1 | tr -d ' \n')
echo "level3: random=$RANDOM_HEX" >> results/level3.txt
echo "level3: done"
