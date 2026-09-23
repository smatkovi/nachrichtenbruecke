#!/bin/sh
# Setzt Dienst und Symbol der vier Bruecke-Konten.
#
# Ohne diese beiden Eigenschaften zeigen die Kontoliste und die
# Nachrichten-App bei einem Gespraech nicht an, ueber welchen Dienst es
# laeuft -- das Feld bleibt einfach leer. Der Kontoverwalter uebernimmt
# sie nicht aus der .manager-Datei; sie gehoeren zum Konto, nicht zum
# Protokoll, und werden beim Anlegen mitgegeben oder eben nachgetragen.
#
# Auf dem Geraet laufen lassen, in der Benutzersitzung.
set -e
AM=org.freedesktop.Telepathy.AccountManager
IF=org.freedesktop.Telepathy.Account

setze() {
    dbus-send --session --print-reply --dest=$AM "$1" \
        org.freedesktop.DBus.Properties.Set \
        string:$IF string:"$2" variant:string:"$3" >/dev/null
}

for P in whatsapp signal telegram matrix; do
    A=/org/freedesktop/Telepathy/Account/bruecke/$P/${P}0
    if dbus-send --session --print-reply --dest=$AM "$A" \
        org.freedesktop.DBus.Properties.Get string:$IF string:Valid >/dev/null 2>&1
    then
        setze "$A" Service "$P"
        setze "$A" Icon "icon-m-service-$P"
        echo "$P: Dienst und Symbol gesetzt"
    else
        echo "$P: kein Konto unter $A" >&2
    fi
done
