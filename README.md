# Brücke

Ein Telepathy-Verbindungsmanager für das Nokia N9/N950, der WhatsApp und
Signal in die Nachrichten-App trägt.

Er tritt **neben** [pybridge](https://openrepos.net/), nicht an seine
Stelle: Telegram und Matrix laufen dort weiter, bis hier etwas steht, dem
man sie anvertrauen mag. Deshalb ein eigener Bus-Name.

## Warum überhaupt neu

pybridge ist in Python geschrieben und spricht rohes D-Bus — keine
Telepathy-Bibliothek, 1700 Zeilen. Das ist kein Vorwurf, es funktioniert.
Aber drei Eigenschaften haben auf diesem Gerät Schaden angerichtet, und
alle drei sind hier anders gelöst:

**Kanäle melden.** pybridge startet dafür einen eigenen Python-Prozess je
Meldung. Beim Nachholen vieler Nachrichten trieb das die Last auf über
sieben. Hier geschieht es im selben Prozess.

**Die Chatliste.** pybridge holt sie genau einmal beim Verbinden. War der
Dienst gerade neu gestartet und antwortete mit null Chats, blieb die
Namenstabelle für die ganze Sitzung leer — in der Nachrichten-App standen
dann rohe Nummern, und wer darin antwortete, schrieb unbemerkt in einen
Einzelchat statt in die Gruppe.

**Der Umweg über Python-Daemons.** pybridge spricht mit den Diensten über
einen Unix-Socket und einen Übersetzer dazwischen. Beide Dienste bieten
dieselbe HTTP-Schnittstelle; der Übersetzer entfällt, und mit ihm zwei
Prozesse.

Dazu der Platz: pybridge braucht rund 9 MB, telepathy-ring in C 2,8.

Und eine Falle weniger: pybridge schlüsselt seine Telepathy-Griffe nach
dem **Anzeigenamen**. Zwei Kontakte gleichen Namens fallen dann zusammen,
und wer seinen Namen ändert, bekommt einen neuen Gesprächsfaden. Hier ist
die Chat-Kennung der Schlüssel, der Name nur eine Beschriftung.

## Vier Protokolle, zwei Wege

WhatsApp und Signal sprechen HTTP mit den Diensten von
[harbour-whatsapp-meego](https://github.com/smatkovi/harbour-whatsapp-meego)
und [Fluesterwind](https://github.com/smatkovi/fluesterwind) — direkt,
ohne Übersetzer dazwischen.

Telegram und Matrix hängen an den vorhandenen Python-Daemons
(`pytelegram`, `pymatrix`) über einen Unix-Socket mit Zeilen-JSON. Die
bleiben, wie sie sind; sie zu ersetzen wäre ein zweites Projekt.

Was das an Platz spart, sobald es trägt:

| | heute | danach |
|---|---|---|
| pybridge | 9 MB | — |
| whatsapp_daemon.py | 6 MB | — |
| signal_daemon.py | 5 MB | — |
| Brücke | — | 1 MB |

Die beiden großen Posten bleiben vorerst: `telegram_daemon.py` mit 55 MB
und `matrix_daemon.py` mit 12 MB.

## Stand

Alle vier Protokolle laufen über die Brücke: WhatsApp 50 Chats, Signal 96,
Telegram 200, Matrix 52. pybridge ist abgeschaltet.

Was das gebracht hat:

| | vorher | nachher |
|---|---|---|
| pybridge | 9,1 MB | — |
| whatsapp_daemon.py | 5,9 MB | — |
| signal_daemon.py | 5,3 MB | — |
| telegram_daemon.py | 57,9 MB | 57,9 MB |
| matrix_daemon.py | 12,5 MB | 12,5 MB |
| Brücke | — | 1,9 MB |
| **zusammen** | **~90 MB** | **~72 MB** |

Die beiden großen Posten bleiben: die Daemons von Telegram und Matrix.
Sie in Rust neu zu schreiben hieße, MTProto und das Matrix-Protokoll
mitzubringen — ein eigenes Projekt, aber danach wären es keine 20 MB
mehr, sondern zwei.

Noch nicht geprüft: ob Nachrichten in der Nachrichten-App ankommen und ob
sich aus ihr heraus senden lässt.

Eine Stolperstelle, die Tage kosten kann: eine verbundene Verbindung
braucht einen **gültigen eigenen Griff**. Bleibt `SelfHandle` null,
antwortet sie zwar auf alles und meldet `GetStatus() == 0`, gilt dem
Kontoverwalter aber trotzdem als nicht verbunden — in der Oberfläche
steht dann „offline", ohne dass irgendwo ein Fehler auftaucht.

## Bauen

    . tools/cross.env
    tools/build.sh          # -> build/bruecke

Was dabei nicht offensichtlich ist, steht in `tools/cross.env`.
