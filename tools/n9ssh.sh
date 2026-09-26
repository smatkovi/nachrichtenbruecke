#!/bin/sh
# SSH to the N9/N950. Their OpenSSH 5.1p1 knows neither ed25519 nor SHA-2
# signatures, so it needs ssh-rsa for both host and user key, and the
# dedicated key.
#
#   N9_HOST=192.168.1.8 tools/n9ssh.sh 'command'
#
# N9_JUMP springt ueber einen Zwischenwirt (etwa arch, wenn das Geraet nur
# im Heimnetz haengt). Der Schluessel bleibt dabei hier: bei ProxyJump
# handelt der eigene Client den letzten Sprung aus, der Zwischenwirt
# leitet nur Bytes weiter.
#
#   N9_JUMP=arch N9_HOST=192.168.1.8 tools/n9ssh.sh 'command'
set -e
[ -n "$N9_JUMP" ] && SPRUNG="-J $N9_JUMP" || SPRUNG=""
exec ssh $SPRUNG -oHostKeyAlgorithms=+ssh-rsa -oPubkeyAcceptedAlgorithms=+ssh-rsa \
    -i "$HOME/.ssh/id_rsa_n9" -oConnectTimeout=15 \
    "${N9_USER:-user}@${N9_HOST:-192.168.1.15}" "$@"
