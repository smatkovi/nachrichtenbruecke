#!/bin/sh
# Wer laesst den Verbindungsdialog aufgehen?
#
#   tools/wer-fragt-nach-netz.sh start    # Mitschnitt anwerfen (laeuft weiter)
#   tools/wer-fragt-nach-netz.sh ende     # Mitschnitt beenden
#   tools/wer-fragt-nach-netz.sh lesen    # Befund zusammenstellen
#
# Laeuft AUF dem Geraet (busybox sh), nicht hier. Er muss das Wegfallen
# des Netzes ueberleben -- genau dann wird gemessen --, also nohup und
# alles in Dateien unter /tmp/netzfrage.
#
# Warum nicht einfach dbus-monitor?  Der System-Bus von Harmattan laesst
# kein Mitlesen von Methodenaufrufen zu (gemessen am 17.10.2026: als root
# sieht dbus-monitor nur die Signale des Busses selbst). Der Aufruf
# `connect_req` an icd2 ist also nicht direkt zu sehen.
#
# Was stattdessen geht: icd2 schreibt bei jedem Zugriff auf seine
# Schnittstelle den Bus-Namen des Fragers ins syslog
# ("Duplicate filter: Do not add filter for app :1.681"). Dieser Name
# allein sagt nichts -- also fuehrt dieses Skript ein Verzeichnis: alle
# drei Sekunden fragt es den Bus nach seinen Namen und schreibt zu jedem
# neuen die Kennzahl und das Programm dahinter. Hinterher laesst sich
# ":1.681" damit in ein Programm uebersetzen, auch wenn es laengst
# beendet ist.
set -e
# Nicht /tmp: das ist auf dem N950 ein tmpfs von 4 MB, und der
# Mitschnitt schreibt stundenlang. Das Heimverzeichnis hat Platz.
ORT=$HOME/netzfrage
BUS=/org/freedesktop/DBus
BUSD=org.freedesktop.DBus

zeit() { date +%H:%M:%S; }

namen() {
    dbus-send --system --print-reply --dest=$BUSD $BUS $BUSD.ListNames 2>/dev/null \
        | grep -o '":1\.[0-9]*"' | tr -d '"'
}

pid_von() {
    dbus-send --system --print-reply --dest=$BUSD $BUS \
        $BUSD.GetConnectionUnixProcessID string:"$1" 2>/dev/null \
        | grep -o 'uint32 [0-9]*' | cut -d' ' -f2
}

verzeichnis() {
    : > $ORT/bekannt
    while :; do
        # Keine &&-Ketten in diesem Lauf: scheitert das erste Glied, gilt
        # unter `set -e` die ganze Zeile als gescheitert und der
        # Mitschnitt endet still (am 17.10.2026 einmal passiert -- die
        # erste Fassung starb nach dem ersten Namen mit Programm).
        for n in $(namen); do
            if grep -q "^$n " $ORT/bekannt 2>/dev/null; then
                continue
            fi
            echo "$n " >> $ORT/bekannt
            p=$(pid_von "$n")
            exe=""
            if [ -n "$p" ]; then
                exe=$(readlink /proc/$p/exe 2>/dev/null || true)
                if [ -z "$exe" ]; then
                    exe=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null || true)
                fi
            fi
            echo "$(zeit) $n pid=${p:-?} ${exe:-unbekannt}" >> $ORT/wer
        done
        sleep 3
    done
}

# Welches Konto will gerade verbinden? Die Kontoverwaltung sitzt am
# Sitzungsbus, dessen Adresse steht im Umfeld eines laufenden Prozesses.
sitzungsbus() {
    for p in $(ls /proc | grep '^[0-9]*$'); do
        a=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null \
            | grep '^DBUS_SESSION_BUS_ADDRESS=' | head -1)
        [ -n "$a" ] && { echo "${a#DBUS_SESSION_BUS_ADDRESS=}"; return; }
    done
}

konten() {
    DBUS_SESSION_BUS_ADDRESS=$(sitzungsbus)
    export DBUS_SESSION_BUS_ADDRESS
    [ -n "$DBUS_SESSION_BUS_ADDRESS" ] || return 0
    while :; do
        echo "== $(zeit)" >> $ORT/konten
        for k in $(mc-tool list 2>/dev/null); do
            z=$(mc-tool show "$k" 2>/dev/null \
                | grep -E '^ *(Current|Requested|Automatic):' | tr -s ' ' | tr '\n' ' ')
            echo "   $k $z" >> $ORT/konten
        done
        sleep 10
    done
}

# Welches Fenster kommt hoch? Der Verbindungsdialog ist ein X-Fenster
# wie jedes andere, und es traegt die Kennzahl seines Prozesses
# (_NET_WM_PID). Das sagt zwar nicht, *wer* gefragt hat -- den Dialog
# zeigt eine Systemkomponente --, aber es sagt sicher, *welcher* Dialog
# es war, und wann genau. Der Fragesteller steht dann im syslog.
fenster() {
    DISPLAY=:0
    export DISPLAY
    : > $ORT/fensterbekannt
    while :; do
        liste=$(xprop -root _NET_CLIENT_LIST 2>/dev/null | sed 's/.*# //' | tr -d ' ' | tr ',' '\n')
        for w in $liste; do
            if grep -q "^$w$" $ORT/fensterbekannt 2>/dev/null; then
                continue
            fi
            echo "$w" >> $ORT/fensterbekannt
            n=$(xprop -id "$w" WM_NAME 2>/dev/null | sed 's/^WM_NAME[^=]*= //' || true)
            p=$(xprop -id "$w" _NET_WM_PID 2>/dev/null | sed 's/.*= //' || true)
            e=""
            if [ -n "$p" ]; then
                e=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null || true)
            fi
            echo "$(zeit) $w pid=${p:-?} ${n:-ohne Namen} ${e:-}" >> $ORT/fenster
        done
        sleep 2
    done
}

case "${1:-start}" in
start)
    mkdir -p $ORT
    : > $ORT/wer; : > $ORT/konten; : > $ORT/fenster
    echo "$(date) Mitschnitt an" > $ORT/beginn
    # Die Stelle im syslog festhalten, ab der gemessen wird.
    wc -l < /var/log/syslog > $ORT/syslog.zeile
    nohup sh "$0" verzeichnis-lauf >/dev/null 2>&1 &
    echo $! > $ORT/pid.wer
    nohup sh "$0" konten-lauf >/dev/null 2>&1 &
    echo $! > $ORT/pid.konten
    nohup sh "$0" fenster-lauf >/dev/null 2>&1 &
    echo $! > $ORT/pid.fenster
    # Wachhund: nach zwei Stunden ist Schluss, auch wenn niemand
    # aufraeumt. Ein Mitschnitt, der vergessen wird, laeuft sonst bis
    # zum naechsten Neustart und schreibt dabei zu (etwa 20 kB/min).
    nohup sh -c "sleep 7200; sh '$0' ende" >/dev/null 2>&1 &
    sleep 4
    echo "laeuft:"; cat $ORT/pid.wer $ORT/pid.konten $ORT/pid.fenster
    echo "erste Zeilen:"; head -3 $ORT/wer
    ;;
verzeichnis-lauf) verzeichnis ;;
konten-lauf) konten ;;
fenster-lauf) fenster ;;
ende)
    for f in $ORT/pid.wer $ORT/pid.konten $ORT/pid.fenster; do
        [ -f "$f" ] && kill "$(cat $f)" 2>/dev/null || true
    done
    echo "beendet"
    ;;
lesen)
    ab=$(cat $ORT/syslog.zeile 2>/dev/null || echo 1)
    echo "=== icd2 im syslog ab Beginn des Mitschnitts"
    tail -n +"$ab" /var/log/syslog | grep -i "icd2\|wlancond\|gprs" | tail -40
    echo "=== Bus-Namen und ihre Programme"
    cat $ORT/wer 2>/dev/null | tail -60
    echo "=== Fenster, die neu aufgingen"
    cat $ORT/fenster 2>/dev/null | tail -30
    echo "=== Konten (Anfang und Ende)"
    head -30 $ORT/konten 2>/dev/null; echo "   ..."; tail -30 $ORT/konten 2>/dev/null
    ;;
*) echo "start | ende | lesen"; exit 1 ;;
esac
