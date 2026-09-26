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

## Fünf Protokolle, zwei Wege

WhatsApp, Signal und **Briar** sprechen HTTP mit den Diensten von
[harbour-whatsapp-meego](https://github.com/smatkovi/harbour-whatsapp-meego)
und [Fluesterwind](https://github.com/smatkovi/fluesterwind) — direkt,
ohne Übersetzer dazwischen. Briar kam zuletzt dazu (harbour-briar, Dienst
`briard` auf Port 8105) und kostete hier drei Zeilen: sein Dienst beantwortet
dieselben vier Wege. Eine Kennung ist dort `c<Kontakt>` oder `g<Gruppe>`.
Einrichten auf dem Gerät: `tools/briar-einrichten.sh`.

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

Eine zweite, die eine Stunde kostete: der Griff auf den Daemon bleibt
liegen, wenn der Daemon stirbt. Die Bruecke meldete das Konto dann bis
zu ihrem eigenen Neustart als verbunden — die Kontenwache sieht ein
verbundenes Konto und stoesst nichts mehr an, und im Protokoll steht
nur `Zustand: 2 / Zustand: 0`. Deshalb prueft `Verbinden` den Griff mit
`Hintergrund::lebt()`, bevor es ihn wiederverwendet.

Eine dritte, die einen Nachmittag kostete: **abschalten hielt nicht.** Ein
Konto in der Kontenübersicht offline zu stellen wirkte, und beim nächsten
Start der Brücke war es wieder oben. Zwei Ursachen, und beide muss man am
Konto nachlesen statt am Aufruf.

`Connection.Disconnect` kommt von Mission Control bei jeder Gelegenheit —
beim Bildschirmschlaf etwa. Die Verbindung zum Daemon dabei abzubauen
hieße, jede eingehende Nachricht zu verpassen, bis jemand die App öffnet;
deshalb hielt die Brücke den Draht und meldete nur „getrennt". Nur: das
Abschalten durch einen Menschen kommt als **derselbe** Aufruf an.
Unterscheiden lässt sich das allein an `RequestedPresence` des Kontos —
die Gelegenheitstrennung lässt sie auf 2 stehen, die Entscheidung setzt
sie auf 1. Nachgemessen: Mission Control nimmt `Disconnect` (nicht
`SetPresence`) und setzt die Eigenschaft **vorher**.

Schwerer wog die zweite: Mission Control ruft `Connect()` beim Hochkommen
der Brücke für jedes Konto, das „automatisch verbindet" — und nimmt dafür
die **automatische** Anwesenheit, nicht die gewünschte. Am N9 standen
darum alle fünf Konten gleichzeitig auf `Requested: offline (1)` und
`Current: available (2)`. Ändern lässt sich der Kontoverwalter nicht
(geschlossen, von aegis gehalten), also prüft die Brücke auch beim
Verbinden und verweigert ein abgeschaltetes Konto. Das setzt sich nicht
fest: Mission Control gibt nach zwei Runden auf.

Beides geht durch `kontenwache::will_offline()`, und die meldet „offline"
nur bei klarem Befund. Busfehler, unlesbarer Wert, kein Konto gefunden:
dann bleibt der Draht. Ein zu Unrecht abgebauter kostet Nachrichten, ein
zu Unrecht gehaltener nur Strom.

## Bauen

    . tools/cross.env
    tools/build.sh          # -> build/bruecke

Was dabei nicht offensichtlich ist, steht in `tools/cross.env`.

## Einspielen

    N9_HOST=192.168.1.15 tools/einspielen.sh               # am Draht
    N9_JUMP=arch N9_HOST=192.168.1.8 tools/einspielen.sh   # über einen Zwischenwirt

Die Brücke kam nie aus einem Paket, also steht ihr Hash nicht in aegis'
`refhashlist` und Kopieren ist harmlos — anders als bei allem, was dpkg
gelegt hat.

Zwei Fallen stecken in diesem Skript, beide bezahlt: `pkill -x bruecke`
**trifft den Prozess nicht** (der 2.6.32-Kernel hat kein
`/proc/<pid>/comm`), und wer vor dem Ersetzen abschießt, bekommt binnen
einer Sekunde eine neue Brücke aus der **alten** Datei — die Datei ist
dann neu, das laufende Programm nicht. Also erst ersetzen, dann über die
`/proc/*/exe`-Verweise suchen und beenden; ein `(deleted)` dahinter
verrät das alte Inode. Gestartet wird nichts von Hand: das Skript
aktiviert den Busnamen, damit die Konten wieder verbinden.
