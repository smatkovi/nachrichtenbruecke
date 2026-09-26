#!/bin/sh
# Spielt build/bruecke auf ein Geraet und laesst es neu starten.
#
#   N9_HOST=172.28.172.2 tools/einspielen.sh               # N9 am Draht
#   N9_JUMP=arch N9_HOST=192.168.1.8 tools/einspielen.sh   # N950 im Heimnetz
#
# NOCH NICHT GELAUFEN (26.09.2026): geschrieben, als kein Geraet erreichbar
# war. Zwei Stellen koennen haken -- sudo fragt am N9 vielleicht nach einem
# Passwort (nur fuer das N950 ist passwortloses sudo vermerkt), und das
# Beenden der Bruecke laesst Drahtpost und Maschendraht als Waisen laufen;
# ob die neu gestartete Bruecke deren Socket wiederfindet oder einen zweiten
# Daemon anfaengt, ist hier nicht geprueft.
#
# Kopieren ist hier harmlos: /opt/bruecke/bruecke kam nie aus einem Paket,
# also steht sein Hash nicht in aegis' refhashlist und niemand vergleicht
# ihn. Bei allem, was dpkg gelegt hat, waere das der Weg in ein
# "Operation not permitted" -- dort gehoert aegis-dpkg -i hin.
set -e
cd "$(dirname "$0")/.."
: "${N9_HOST:=192.168.1.15}"
export N9_HOST
[ -x build/bruecke ] || { echo "build/bruecke fehlt -- erst tools/build.sh"; exit 1; }

echo "== $N9_HOST"
[ -n "$N9_JUMP" ] && SPRUNG="-J $N9_JUMP" || SPRUNG=""
scp $SPRUNG -oHostKeyAlgorithms=+ssh-rsa -oPubkeyAcceptedAlgorithms=+ssh-rsa \
    -i "$HOME/.ssh/id_rsa_n9" -oConnectTimeout=15 \
    build/bruecke "${N9_USER:-user}@$N9_HOST:/tmp/bruecke.neu"

# Erst ersetzen, dann beenden -- nicht umgekehrt.
#
# Andersherum sah es am 26.09.2026 so aus, als waere eingespielt worden:
# nach dem pkill aktiviert Mission Control die Bruecke binnen einer
# Sekunde neu, und zwar noch aus der ALTEN Datei. Danach zeigte
# /opt/bruecke/bruecke den neuen Hash, /proc/<pid>/exe aber den alten --
# der Prozess hielt das abgehaengte Inode fest. Ersetzen geht auch im
# Betrieb: install haengt die alte Datei aus und legt eine neue hin
# (kein ETXTBSY), der laufende Prozess merkt davon nichts. Erst der
# Abschuss danach ist wirksam.
#
# Gesucht wird der Prozess ueber ps und den Pfad, nicht mit pkill -x:
# dessen Namensvergleich trifft die Bruecke auf Harmattan nicht (der
# 2.6.32-Kernel hat kein /proc/<pid>/comm). Am 26.09.2026 lief sie deshalb
# zweimal unbemerkt weiter, mit dem abgehaengten alten Inode -- die Datei
# war neu, das Programm nicht.
#
# Gestartet wird nichts von Hand: der Sitzungsbus aktiviert die Bruecke
# beim naechsten Zugriff, und sie holt Drahtpost und Maschendraht selbst.
sh tools/n9ssh.sh '
    a=""
    sudo install -m 755 /tmp/bruecke.neu /opt/bruecke/bruecke || {
        echo "== install gescheitert, nichts angetastet"; exit 1
    }
    rm -f /tmp/bruecke.neu
    echo "== gelegt:"; md5sum /opt/bruecke/bruecke

    # Wer laeuft aus /opt/bruecke/bruecke? Ueber die exe-Verweise, nicht
    # ueber pkill: dessen Namensvergleich trifft die Bruecke hier nicht
    # (der 2.6.32-Kernel hat kein /proc/<pid>/comm). Ein " (deleted)"
    # hinter dem Pfad heisst: der Prozess haelt noch das alte Inode.
    laufende() {
        for d in /proc/[0-9]*; do
            case "$(readlink $d/exe 2>/dev/null)" in
                /opt/bruecke/bruecke*) echo "${d#/proc/}";;
            esac
        done
    }
    for pid in $(laufende); do
        echo "== beende $pid ($(readlink /proc/$pid/exe))"
        kill $pid || true
    done
    sleep 3

    # Nicht ohne Bruecke dastehen lassen. Von Hand gestartet waere sie
    # ohne Sitzungsbus; also den Bus aus einem laufenden Prozess holen und
    # den Namen regulaer aktivieren, damit die Konten wieder verbinden.
    # Die Umleitung steht VOR dem "<": sonst meldet die Shell den
    # verweigerten Zugriff auf fremde /proc-Eintraege noch auf dem alten
    # stderr, und mit set -e endet der Block, bevor die Bruecke hochkommt.
    for d in /proc/[0-9]*; do
        v=$(tr "\0" "\n" 2>/dev/null < $d/environ | sed -n "s/^DBUS_SESSION_BUS_ADDRESS=//p")
        if [ -n "$v" ]; then a=$v; break; fi
    done
    if [ -n "$a" ]; then
        DBUS_SESSION_BUS_ADDRESS=$a dbus-send --session \
            --dest=org.freedesktop.DBus / \
            org.freedesktop.DBus.StartServiceByName \
            string:org.freedesktop.Telepathy.ConnectionManager.bruecke \
            uint32:0 >/dev/null 2>&1 || true
        sleep 5
    else
        echo "== kein Sitzungsbus gefunden; die Bruecke startet beim naechsten Zugriff"
    fi

    for pid in $(laufende); do
        echo "== laeuft: $pid"; md5sum /proc/$pid/exe
    done
'
echo "== fertig"
