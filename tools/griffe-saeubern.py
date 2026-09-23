#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""Entfernt Griffe, die auf einen Anzeigenamen oder eine Kontaktliste zeigen.

Beides sind Folgen desselben Fehlers: InspectHandles gab den Anzeigenamen
zurueck, und RequestHandles nahm jede Zeichenkette als Kennung -- auch die
Listennamen, die der Kontoverwalter mit Griffart 3 erfragt. Wer so einen
Griff als Gespraech oeffnet, schickt an "Fabian Mistelberger" statt an eine
Kennung.

Gelaeuchte Griffe sind nicht wiederverwendbar: naechster bleibt, wo er ist,
damit kein alter Gespraechsfaden eine neue Bedeutung bekommt.

    python tools/griffe-saeubern.py [--wirklich] <datei> [...]
"""
import json
import sys

LISTEN = ("stored", "publish", "subscribe", "deny", "known", "allow", "hide")


def saeubern(pfad, wirklich):
    k = json.load(open(pfad))
    zu_griff = k.get("zu_griff", {})
    zu_kennung = k.get("zu_kennung", {})
    namen = k.get("namen", {})
    anzeigenamen = set(namen.values())

    weg = {}
    for kennung, griff in zu_griff.items():
        eigener = namen.get(str(griff), "")
        if kennung in LISTEN:
            weg[kennung] = (griff, "Kontaktliste")
        elif kennung in anzeigenamen and kennung != eigener:
            weg[kennung] = (griff, "Anzeigename eines anderen Griffs")

    print("%s: %d Griffe, %d zu entfernen" % (pfad, len(zu_griff), len(weg)))
    for kennung, (griff, grund) in sorted(weg.items()):
        print("   %-4d %-28s %s" % (griff, repr(kennung), grund))
    if not weg or not wirklich:
        return
    for kennung, (griff, _) in weg.items():
        zu_griff.pop(kennung, None)
        zu_kennung.pop(str(griff), None)
        namen.pop(str(griff), None)
    open(pfad, "w").write(json.dumps(k))
    print("   geschrieben")


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if a != "--wirklich"]
    saeubern_wirklich = "--wirklich" in sys.argv[1:]
    for pfad in args:
        saeubern(pfad, saeubern_wirklich)
