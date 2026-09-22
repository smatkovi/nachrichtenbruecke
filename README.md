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

## Stand

Früh. Der Manager läuft auf dem Gerät, wird über D-Bus aktiviert und
meldet seine Protokolle. Verbindungen und Kanäle fehlen noch.

## Bauen

    . tools/cross.env
    tools/build.sh          # -> build/bruecke

Was dabei nicht offensichtlich ist, steht in `tools/cross.env`.
