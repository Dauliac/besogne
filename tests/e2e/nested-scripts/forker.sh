#!/bin/sh
# Demonstrates real fork+exec: spawns distinct named worker processes.
# Each worker is a separate sh -c invocation (not a subshell).
set -e

OUTFILE="results/forked.txt"

# Fork 3 named workers with different commands
sh -c 'sleep 0.01; echo "worker-alpha: done (pid=$$)"' >> "$OUTFILE" &
PID1=$!

sh -c 'sleep 0.01; echo "worker-beta: done (pid=$$)"' >> "$OUTFILE" &
PID2=$!

sh -c 'sleep 0.01; echo "worker-gamma: done (pid=$$)"' >> "$OUTFILE" &
PID3=$!

# Also fork a nested chain: parent spawns child which spawns grandchild
sh -c '
  echo "chain-parent: start (pid=$$)" >> '"$OUTFILE"'
  sh -c "sleep 0.01; echo \"chain-child: done (pid=\$\$)\"" >> '"$OUTFILE"'
  echo "chain-parent: done (pid=$$)" >> '"$OUTFILE"'
' &
PID4=$!

wait $PID1 $PID2 $PID3 $PID4
echo "forker: all workers done"
