#!/bin/sh
# Setzt Dienst, Symbol und Beschriftung der Bruecke-Konten.
#
# Ohne diese Eigenschaften zeigen die Kontoliste und die Nachrichten-App
# bei einem Gespraech nicht an, ueber welchen Dienst es laeuft -- das Feld
# bleibt einfach leer. Der Kontoverwalter uebernimmt sie nicht aus der
# .manager-Datei; sie gehoeren zum Konto, nicht zum Protokoll, und werden
# beim Anlegen mitgegeben oder eben nachgetragen.
#
# Der Pfad wird gesucht, nicht geraten. Frueher stand hier fest
# ".../bruecke/$P/${P}0" -- das Briar-Konto heisst aber briar1 (briar0 gab
# es einmal und ist weg), und so lief das Skript jedes Mal mit
# "briar: kein Konto" durch, waehrend genau dieses Konto unbeschriftet
# blieb.
#
# Auf dem Geraet laufen lassen, in der Benutzersitzung.
set -e
AM=org.freedesktop.Telepathy.AccountManager
AMP=/org/freedesktop/Telepathy/AccountManager
IF=org.freedesktop.Telepathy.Account

setze() {
    dbus-send --session --print-reply --dest=$AM "$1" \
        org.freedesktop.DBus.Properties.Set \
        string:$IF string:"$2" variant:string:"$3" >/dev/null
}

lies() {
    dbus-send --session --print-reply --dest=$AM "$1" \
        org.freedesktop.DBus.Properties.Get string:$IF string:"$2" 2>/dev/null \
        | sed -n 's/.*string "\(.*\)".*/\1/p'
}

# Alle Konten des Managers "bruecke", so wie sie wirklich heissen.
konten() {
    dbus-send --session --print-reply --dest=$AM $AMP \
        org.freedesktop.DBus.Properties.Get \
        string:org.freedesktop.Telepathy.AccountManager string:ValidAccounts \
        | sed -n 's|.*"\(/org/freedesktop/Telepathy/Account/bruecke/[^"]*\)".*|\1|p'
}

beschriftung() {
    case "$1" in
        whatsapp) echo "WhatsApp";;
        signal)   echo "Signal";;
        telegram) echo "Telegram";;
        matrix)   echo "Matrix";;
        briar)    echo "Briar";;
        *)        echo "$1";;
    esac
}

gefunden=0
for A in $(konten); do
    # .../Account/bruecke/<protokoll>/<kennung>
    P=$(echo "$A" | sed 's|.*/bruecke/\([^/]*\)/.*|\1|')
    gefunden=$((gefunden + 1))
    setze "$A" Service "$P"
    setze "$A" Icon "icon-m-service-$P"
    # Die Beschriftung nur setzen, wo keine steht: einen selbst
    # vergebenen Namen soll das Skript nicht ueberschreiben.
    N=$(lies "$A" Nickname)
    if [ -z "$N" ]; then
        setze "$A" Nickname "$(beschriftung "$P")"
        echo "$P: Dienst, Symbol und Beschriftung gesetzt ($A)"
    else
        echo "$P: Dienst und Symbol gesetzt, Beschriftung war schon \"$N\" ($A)"
    fi
done

[ "$gefunden" -gt 0 ] || { echo "kein einziges bruecke-Konto gefunden" >&2; exit 1; }
