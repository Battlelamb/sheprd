#!/usr/bin/env bash
# Copies a push relay's secrets, made by make-secrets.sh, to its droplet over
# SSH and (re)starts the relay there:
#
#   deploy/push-relay/install-secrets.sh ~/push-sandbox root@203.0.113.7
#
# They replace the droplet's secrets as a whole, once its start.sh has
# checked them: it refuses a missing file or an APNs key for the other
# environment, and leaves the running relay and its secrets alone then.
set -euo pipefail

[ "$#" -eq 2 ] || { echo "usage: $0 DIR USER@HOST" >&2; exit 2; }
dir=$1 host=$2
files=(apns/config.json apns/AuthKey.p8 relay-key.pem credential-secret)
for file in "${files[@]}"; do
  [ -f "$dir/$file" ] || { echo "no $dir/$file; make them with make-secrets.sh" >&2; exit 2; }
done
# The key before a rotation, so handles sealed to it still open.
if [ -f "$dir/relay-key-previous.pem" ]; then
  files+=(relay-key-previous.pem)
fi

# Into a fresh directory first, so a dropped connection leaves the old
# secrets in place; start.sh moves them over once they check out. macOS tar
# adds no AppleDouble files with COPYFILE_DISABLE.
COPYFILE_DISABLE=1 tar -C "$dir" -cf - "${files[@]}" |
  ssh "$host" 'set -eu
    cd /opt/sheprd-push-relay
    umask 077
    rm -rf secrets.new
    mkdir secrets.new
    tar -C secrets.new --no-same-owner -xf -
    NEW_SECRETS=1 ./start.sh'
