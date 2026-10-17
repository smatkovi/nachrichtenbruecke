//! Ist ueberhaupt ein Netz da?
//!
//! Ohne diese Frage versucht die Bruecke auch dann zu verbinden, wenn
//! das Geraet gar nicht am Netz haengt: sie startet die Daemons, die
//! laufen in ihre eigenen Fristen (Drahtpost wartet bis zu 30 s auf
//! seinen Socket, die Chatliste noch einmal bis zu 24 s), melden
//! schliesslich einen Fehler, und die Kontoverwaltung faengt von vorne
//! an. Das ist der Leerlauf, den man als Mensch daran merkt, dass das
//! Telefon warm wird und die Konten zwischen „verbindet" und
//! „getrennt" flackern.
//!
//! Gefragt wird nicht icd2, sondern die Weiterleitungstabelle des Kerns.
//! Das hat zwei Gruende: icd2 beantwortet `state_req` mit der Anzahl der
//! Verbindungen, die es *verwaltet*, und das ist nicht dasselbe wie „es
//! geht etwas hinaus" (eine fremd aufgebaute Strecke zaehlt dort nicht);
//! und jede Frage an icd2 legt dort einen Filter auf den eigenen
//! Bus-Namen an. Die Tabelle dagegen ist eine Datei, kostet nichts und
//! luegt nicht.
//!
//! Im Zweifel ist das Netz da. Wer die Tabelle nicht lesen kann, soll
//! verbinden duerfen -- ein zu Unrecht verweigerter Draht kostet
//! Nachrichten, ein zu Unrecht versuchter nur einen Fehlschlag.

/// Gibt es eine Vorgabestrecke ueber ein echtes Gegenstueck?
pub fn netz_da() -> bool {
    match std::fs::read_to_string("/proc/net/route") {
        Ok(inhalt) => vorgabestrecke(&inhalt),
        Err(_) => true,
    }
}

/// Die Tabelle lesen: Spalte 1 ist das Gegenstueck, Spalte 2 das Ziel in
/// Hexadezimal. `00000000` heisst „alles andere", also Vorgabestrecke.
///
/// Steht sie auf `lo`, ist sie keine: so sieht die Tabelle aus, wenn
/// jemand den Rueckschleifen-Vermittler eingerichtet hat, aber nichts
/// nach draussen geht.
pub fn vorgabestrecke(tabelle: &str) -> bool {
    for zeile in tabelle.lines().skip(1) {
        let mut spalten = zeile.split_whitespace();
        let Some(gegenstueck) = spalten.next() else {
            continue;
        };
        let Some(ziel) = spalten.next() else {
            continue;
        };
        if ziel.eq_ignore_ascii_case("00000000") && gegenstueck != "lo" {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod proben {
    use super::vorgabestrecke;

    const KOPF: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT";

    #[test]
    fn wlan_ist_netz() {
        let t = format!(
            "{KOPF}\nwlan0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\t0\t0\t0\n"
        );
        assert!(vorgabestrecke(&t));
    }

    #[test]
    fn gprs_ist_netz() {
        let t = format!(
            "{KOPF}\ngprs0\t00000000\t00000000\t0001\t0\t0\t0\t00000000\t0\t0\t0\n"
        );
        assert!(vorgabestrecke(&t));
    }

    /// Mobile Daten aus, kein WLAN: nur noch die Strecke ins eigene
    /// Netz, keine Vorgabestrecke.
    #[test]
    fn ohne_verbindung_kein_netz() {
        let t = format!(
            "{KOPF}\nlo\t0000007F\t00000000\t0001\t0\t0\t0\t000000FF\t0\t0\t0\n"
        );
        assert!(!vorgabestrecke(&t));
    }

    /// Der Rueckschleifen-Vermittler allein ist kein Netz.
    #[test]
    fn lo_zaehlt_nicht() {
        let t = format!(
            "{KOPF}\nlo\t00000000\t00000000\t0001\t0\t0\t0\t00000000\t0\t0\t0\n"
        );
        assert!(!vorgabestrecke(&t));
    }

    #[test]
    fn leere_tabelle_kein_netz() {
        assert!(!vorgabestrecke(&format!("{KOPF}\n")));
    }
}
