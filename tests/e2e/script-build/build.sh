#!/bin/sh
set -e
echo "building $PROJECT_NAME v$PROJECT_VERSION"
cat src/app.txt > dist/build.log
echo '{"status":"ok","name":"'"$PROJECT_NAME"'","version":"'"$PROJECT_VERSION"'"}' > dist/result.json
