#!/bin/sh
# Richtet Briar in der Bruecke ein -- auf dem Geraet laufen lassen, als
# Benutzer, mit gesetztem DBUS_SESSION_BUS_ADDRESS.
#
#   sh briar-einrichten.sh
#
# Voraussetzung: bruecke (neu), bruecke.manager, briar.provider und
# briar.service liegen in /home/user, und briard laeuft (Port 8105).
set -e

BUS=$(tr '\0' '\n' < /proc/$(pgrep -f "[d]rahtpost|[b]ruecke/bruecke" | head -1)/environ 2>/dev/null \
      | grep ^DBUS_SESSION_BUS_ADDRESS= | head -1)
[ -n "$BUS" ] && export "$BUS"
[ -n "$DBUS_SESSION_BUS_ADDRESS" ] || { echo "kein Sitzungsbus gefunden" >&2; exit 1; }

echo "== Dateien legen (braucht root)"
echo "" | sudo -S sh -c '
    install -m 755 /home/user/bruecke-neu /opt/bruecke/bruecke
    install -m 644 /home/user/bruecke.manager /usr/share/telepathy/managers/bruecke.manager
    install -m 644 /home/user/briar.provider /usr/share/accounts/providers/
    install -m 644 /home/user/briar.service /usr/share/accounts/services/
'

echo "== Bruecke neu starten (sie wird bei Bedarf wieder aktiviert)"
kill $(pgrep -f "[b]ruecke/bruecke") 2>/dev/null || true

# Der Kontoverwalter liest die Manager-Datei nur beim Start. Und Vorsicht:
# sein Prozessname ist auf 15 Zeichen gekuerzt, "pkill -x mission-control-5"
# trifft ihn NICHT -- das hat hier eine Stunde gekostet.
echo "== Kontoverwalter neu starten"
kill $(pgrep -f "telepathy/mission-control-5" | head -1) 2>/dev/null || true
sleep 12

echo "== Konto anlegen"
python2.6 - <<'PY'
import dbus
AM = "org.freedesktop.Telepathy.AccountManager"
bus = dbus.SessionBus()
am = dbus.Interface(bus.get_object(AM, "/org/freedesktop/Telepathy/AccountManager"), AM)
try:
    pfad = am.CreateAccount(
        "bruecke", "briar", "Briar (Bruecke)",
        dbus.Dictionary({"account": "briar"}, signature="sv"),
        dbus.Dictionary({
            "org.freedesktop.Telepathy.Account.Enabled": dbus.Boolean(True),
            "org.freedesktop.Telepathy.Account.ConnectAutomatically": dbus.Boolean(True),
            "org.freedesktop.Telepathy.Account.Service": "briar",
            "org.freedesktop.Telepathy.Account.Icon": "icon-m-service-briar",
        }, signature="sv"))
    print("angelegt:", pfad)
except Exception as e:
    print("Fehler:", str(e)[-140:])
PY
