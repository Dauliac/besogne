#!/bin/sh
set -e

echo "level1: start (pid=$$)"
(sleep 0.1 && echo "background: done (pid=$$)" >> results/background.txt) &
BG_PID=$!
sh ./level2.sh
sh ./forker.sh
echo "level1: wrote" >> results/level1.txt
wait $BG_PID
echo "level1: background finished"
echo "hello-from-pipe" | tr 'a-z' 'A-Z' | tee results/pipe.txt > /dev/null
echo "level1: done"
