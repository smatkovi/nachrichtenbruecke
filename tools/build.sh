#!/bin/sh
# Baut die Bruecke fuer das N9/N950.
set -e
cd "$(dirname "$0")/.."
FERN=/tmp/bruecke-src
HOST=$(sh "$HOME/ps/nfsshift-sfos/tools/buildhost.sh")
echo "== Build-Rechner: $HOST"
rsync -a --delete --exclude build --exclude target --exclude .git ./ "$HOST:$FERN/"
ssh "$HOST" 'sh /tmp/bruecke-src/tools/remote-build.sh'
mkdir -p build
scp -q "$HOST:$FERN/build/bruecke" build/
echo "== bruecke fertig ($(stat -c %s build/bruecke) B)"
