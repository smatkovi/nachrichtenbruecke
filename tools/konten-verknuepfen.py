#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""Verknuepft Telepathy-Konten mit ihrer Zeile in accounts.db.

Warum das noetig ist: die Erreichbarkeitsliste beschriftet eine Zeile
nicht ueber Telepathy, sondern ueber libaccounts. Sie sucht in
~/.accounts/accounts.db die Settings-Zeile mit key='tmc-uid', deren Wert
dem Kontonamen entspricht ("bruecke/briar/briar1", also der Pfad ohne
/org/freedesktop/Telepathy/Account/), nimmt den Dienst dieser Zeile und
zeigt dessen Anzeigenamen. Fehlt die Zeile, bleibt die Beschriftung
**leer** -- und zwar stumm.

Account.Service, Account.Icon, Account.DisplayName und Account.Nickname
am Telepathy-Konto zu setzen aendert daran nichts; das ist nachgemessen.
Dafuer ist tools/konten-beschriften.sh zustaendig, und das ist eine
andere Baustelle.

In die Falle laeuft jedes Konto aus AccountManager.CreateAccount -- also
alle fuenf Bruecke-Konten (tools/briar-einrichten.sh legt sie so an).
Ueber die Einstellungen angelegte Konten bekommen ihre Zeile mit; am
N950 zeigten deshalb lange nur die beiden SIP-Konten einen Namen.

Angefasst werden nur Konten, deren Protokoll ueberhaupt einen Dienst in
libaccounts hat -- ring/tel und mmscm/mms sollen keine Zeile bekommen.
Zeigt eine vorhandene Zeile des richtigen Anbieters ins Leere (auf ein
Konto, das es nicht mehr gibt), wird sie umgehaengt statt verdoppelt:
genau so hingen die vier Zeilen von WhatsApp, Telegram, Matrix und
Signal noch an den abgeschalteten pybridge-Konten.

Auf dem Geraet laufen lassen, in der Benutzersitzung:

    python tools/konten-verknuepfen.py              # nur zeigen
    python tools/konten-verknuepfen.py --wirklich   # schreiben

Mit --db <pfad> auf einer Kopie, um den Schreibweg zu proben, ohne die
echte Datenbank anzufassen; die beiden Leser werden dann nicht
angeruehrt.
"""
import os
import shutil
import sqlite3
import sys
import time

import dbus

DB = os.path.expanduser("~/.accounts/accounts.db")
VORSATZ = "/org/freedesktop/Telepathy/Account/"
AM = "org.freedesktop.Telepathy.AccountManager"
AMP = "/org/freedesktop/Telepathy/AccountManager"

# Die beiden Prozesse, die die Konten im Speicher halten. Ein roher
# Schreibzugriff auf die Datenbank loest kein libaccounts-Signal aus,
# also merkt von sich aus keiner etwas.
#   sysuid    -- die Zusammenfassungszeile im Statusmenue (Upstart,
#                respawn, nach dem Abschuss in zwei Sekunden zurueck)
#   presenced -- die aufgeklappte Liste; eigener Prozess, ueber
#                com.nokia.Presence-ui.service per D-Bus aktiviert
LESER = ("/usr/bin/presenced", "/usr/bin/sysuid")

# Schreibweisen, die capitalize() nicht trifft. Der Name landet in der
# Kontoliste, "Whatsapp" waere dort sichtbar falsch.
NAMEN = {
    "whatsapp": "WhatsApp",
    "sip": "SIP",
}


def konten_von_telepathy():
    """Die eingeschalteten Konten als (Name, Protokoll), Name ohne Vorsatz.

    Abgeschaltete bleiben draussen, und das ist keine Feinheit: in der
    Liste stehen sie ohnehin nicht, und das N950 fuehrt mehr als eines
    davon mit -- ein zweites Jabber-Konto und die vier alten
    pybridge-Konten. Wer denen eine Zeile anlegt, sammelt Karteileichen
    an und nimmt dem lebenden Konto womoeglich die Zeile weg, die es
    haette erben koennen.
    """
    bus = dbus.SessionBus()
    raus = []
    eig = dbus.Interface(bus.get_object(AM, AMP),
                         "org.freedesktop.DBus.Properties")
    for p in eig.Get(AM, "ValidAccounts"):
        konto = dbus.Interface(bus.get_object(AM, p),
                               "org.freedesktop.DBus.Properties")
        if not bool(konto.Get("org.freedesktop.Telepathy.Account", "Enabled")):
            continue
        name = str(p)
        if name.startswith(VORSATZ):
            name = name[len(VORSATZ):]
        teile = name.split("/")
        if len(teile) == 3:
            raus.append((name, teile[1]))
    return raus


def sicherung():
    stand = time.strftime("%Y%m%d-%H%M%S")
    for endung in ("", "-wal", "-shm"):
        quelle = DB + endung
        if os.path.exists(quelle):
            ziel = "%s.vor-verknuepfen-%s%s" % (DB, stand, endung)
            shutil.copy2(quelle, ziel)
            print("gesichert: %s" % os.path.basename(ziel))


def neustart():
    """Beide Leser abschiessen, damit sie die Datenbank neu lesen.

    Gesucht wird ueber den vollen Pfad in /proc, nicht mit pkill: dessen
    Namensvergleich trifft presenced auf diesem Geraet nicht.
    """
    for pfad in LESER:
        for eintrag in os.listdir("/proc"):
            if not eintrag.isdigit():
                continue
            try:
                zeile = open("/proc/%s/cmdline" % eintrag).read()
            except IOError:
                continue
            if zeile.split("\0")[0] == pfad:
                os.kill(int(eintrag), 15)
                print("%s (%s) beendet -- kommt von selbst wieder"
                      % (os.path.basename(pfad), eintrag))
                break


def main(wirklich, echt=True):
    c = sqlite3.connect(DB)

    dienste = dict(c.execute("select name, id from Services"))
    verknuepft = {}
    for konto, wert in c.execute(
            "select account, value from Settings where key='tmc-uid'"):
        verknuepft[str(wert)] = konto

    lebend = konten_von_telepathy()
    bekannt = set(n for n, _ in lebend)

    zu_tun = []
    for name, protokoll in sorted(lebend):
        if name in verknuepft:
            continue
        if protokoll not in dienste:
            print("%-44s uebersprungen: kein Dienst '%s' in libaccounts"
                  % (name, protokoll))
            continue
        # Eine Zeile desselben Anbieters, die ins Leere zeigt? Dann die.
        waise = None
        for konto, in c.execute("select id from Accounts where provider=?",
                                (protokoll,)):
            zeigt = c.execute(
                "select value from Settings where account=? and key='tmc-uid'",
                (konto,)).fetchone()
            if zeigt is None or str(zeigt[0]) not in bekannt:
                waise = konto
                break
        zu_tun.append((name, protokoll, waise))

    if not zu_tun:
        print("nichts zu tun: jedes Konto hat seine Zeile")
        return 0

    for name, protokoll, waise in zu_tun:
        if waise is None:
            print("%-44s neue Zeile (Anbieter %s)" % (name, protokoll))
        else:
            print("%-44s Zeile %d umhaengen" % (name, waise))
    if not wirklich:
        print("")
        print("Trockenlauf -- mit --wirklich schreiben.")
        return 0

    sicherung()
    for name, protokoll, waise in zu_tun:
        dienst = dienste[protokoll]
        if waise is None:
            beschriftung = NAMEN.get(protokoll, protokoll.capitalize())
            c.execute("insert into Accounts (name, provider, enabled) "
                      "values (?, ?, 1)", (beschriftung, protokoll))
            konto = c.execute("select last_insert_rowid()").fetchone()[0]
            for svc, schluessel, typ, wert in (
                    (0, "enabled", "b", "1"),
                    (0, "name", "s", beschriftung),
                    (dienst, "ConnectAutomatically", "s", "true"),
                    (dienst, "Icon", "s", "icon-m-service-%s" % protokoll),
                    (dienst, "enabled", "b", "1"),
                    (dienst, "tmc-uid", "s", name)):
                c.execute("insert into Settings "
                          "(account, service, key, type, value) "
                          "values (?, ?, ?, ?, ?)",
                          (konto, svc, schluessel, typ, wert))
            print("%-44s angelegt als Zeile %d" % (name, konto))
        else:
            c.execute("update Settings set value=? "
                      "where account=? and key='tmc-uid'", (name, waise))
            c.execute("update Settings set value='1' "
                      "where account=? and key='enabled' and service<>0",
                      (waise,))
            print("%-44s Zeile %d haengt jetzt daran" % (name, waise))
    c.commit()

    if echt:
        neustart()
    else:
        print("(Kopie -- presenced und sysuid bleiben unberuehrt)")
    return 0


if __name__ == "__main__":
    if "--db" in sys.argv:
        DB = sys.argv[sys.argv.index("--db") + 1]
    sys.exit(main("--wirklich" in sys.argv, DB == os.path.expanduser(
        "~/.accounts/accounts.db")))
