//! Kennungen: die Zuordnung zwischen Telepathy-Griffen und Chat-Kennungen.
//!
//! Telepathy adressiert alles ueber "handles" -- kleine Zahlen, die eine
//! Verbindung vergibt. Sie muessen ueber Neustarts hinweg gleich bleiben,
//! sonst zeigt der Verlauf der Nachrichten-App auf ins Leere. Deshalb
//! liegen sie auf der Platte.
//!
//! pybridge schluesselt sie nach dem ANZEIGENAMEN. Das ist eine Falle:
//! zwei Kontakte gleichen Namens fallen dann zusammen, und wer seinen
//! Namen aendert, bekommt einen neuen Griff und damit einen neuen
//! Gespraechsfaden. Hier ist die Chat-Kennung der Schluessel, und der
//! Name nur eine Beschriftung.
//!
//! Wer von aussen nach einem Griff fragt, nennt aber nicht immer die
//! Kennung: die Nachrichten-App hat frueher den Anzeigenamen
//! zurueckbekommen und fragt spaeter mit genau dem wieder an. `aufloesen`
//! faengt das ab, statt einen zweiten Griff auf einen Namen anzulegen --
//! aus dem dann eine Sendung an "Fabian Mistelberger" wuerde.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct Kennungen {
    /// Chat-Kennung -> Griff.
    zu_griff: HashMap<String, u32>,
    /// Griff -> Chat-Kennung.
    #[serde(default)]
    zu_kennung: HashMap<u32, String>,
    naechster: u32,
    /// Griff -> Anzeigename, nur zur Anzeige.
    #[serde(default)]
    namen: HashMap<u32, String>,
    /// "<art>:<name>" -> Griff, fuer alles, was kein Kontakt ist.
    ///
    /// Der Kontoverwalter fragt mit Griffart 3 nach den Kontaktlisten
    /// ("stored", "publish", "subscribe", "deny"). Ihm dafuer einen
    /// Fehler zu geben, bringt ihn zum Wiederholen ohne Ende; ihm einen
    /// Kontaktgriff zu geben, machte die vier Listen zu Gespraechen.
    /// Also ein eigener Raum: eigene Nummern aus demselben Zaehler, aber
    /// getrennt gefuehrt, damit kennung() sie nie zurueckgibt.
    #[serde(default)]
    sonstige: HashMap<String, u32>,
    #[serde(default)]
    sonstige_namen: HashMap<u32, String>,
}

impl Kennungen {
    pub fn laden(protokoll: &str) -> Self {
        let pfad = Self::pfad(protokoll);
        match std::fs::read_to_string(&pfad) {
            Ok(t) => match serde_json::from_str::<Kennungen>(&t) {
                Ok(mut k) => {
                    if k.naechster < 1 {
                        k.naechster = 1;
                    }
                    k
                }
                Err(_) => Self::leer(),
            },
            Err(_) => Self::leer(),
        }
    }

    fn leer() -> Self {
        Kennungen { naechster: 1, ..Default::default() }
    }

    fn pfad(protokoll: &str) -> PathBuf {
        let heim = std::env::var("HOME").unwrap_or_else(|_| "/home/user".into());
        PathBuf::from(heim)
            .join(".local/share/bruecke")
            .join(format!("kennungen-{protokoll}.json"))
    }

    pub fn sichern(&self, protokoll: &str) {
        let pfad = Self::pfad(protokoll);
        if let Some(ordner) = pfad.parent() {
            let _ = std::fs::create_dir_all(ordner);
        }
        if let Ok(t) = serde_json::to_string(self) {
            // Erst daneben, dann umbenennen: eine halb geschriebene Datei
            // wuerde alle Gespraechsfaeden verlieren.
            let tmp = pfad.with_extension("neu");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(&tmp, &pfad);
            }
        }
    }

    /// Der Griff zu einer Kennung ODER einem Anzeigenamen.
    ///
    /// Erst die Kennung, dann der Name, und nur wenn beides nichts
    /// findet ein neuer Griff. Ist der Name mehrdeutig, wird er nicht
    /// benutzt – zwei Leute gleichen Namens duerfen nicht denselben
    /// Gespraechsfaden bekommen.
    pub fn aufloesen(&mut self, text: &str) -> (u32, bool) {
        if let Some(g) = self.zu_griff.get(text) {
            return (*g, false);
        }
        let mut treffer = None;
        for (g, n) in &self.namen {
            if n == text {
                if treffer.is_some() {
                    treffer = None;
                    break;
                }
                treffer = Some(*g);
            }
        }
        match treffer {
            Some(g) => (g, true),
            None => (self.griff(text), false),
        }
    }

    /// Der Griff zu einer Chat-Kennung; legt ihn an, wenn er fehlt.
    pub fn griff(&mut self, kennung: &str) -> u32 {
        if let Some(g) = self.zu_griff.get(kennung) {
            return *g;
        }
        let g = self.naechster.max(1);
        self.naechster = g + 1;
        self.zu_griff.insert(kennung.to_string(), g);
        self.zu_kennung.insert(g, kennung.to_string());
        g
    }

    /// Ein Griff fuer etwas, das kein Kontakt ist (Griffart != 1).
    pub fn sonstiger_griff(&mut self, art: u32, name: &str) -> u32 {
        let schluessel = format!("{art}:{name}");
        if let Some(g) = self.sonstige.get(&schluessel) {
            return *g;
        }
        let g = self.naechster.max(1);
        self.naechster = g + 1;
        self.sonstige.insert(schluessel, g);
        self.sonstige_namen.insert(g, name.to_string());
        g
    }

    /// Wie ein solcher Griff heisst; fuer InspectHandles.
    pub fn sonstiger_name(&self, griff: u32) -> Option<&str> {
        self.sonstige_namen.get(&griff).map(|s| s.as_str())
    }

    pub fn kennung(&self, griff: u32) -> Option<&str> {
        self.zu_kennung.get(&griff).map(|s| s.as_str())
    }

    pub fn name_setzen(&mut self, griff: u32, name: &str) {
        if !name.is_empty() {
            self.namen.insert(griff, name.to_string());
        }
    }

    pub fn name(&self, griff: u32) -> String {
        self.namen
            .get(&griff)
            .cloned()
            .or_else(|| self.zu_kennung.get(&griff).cloned())
            .unwrap_or_default()
    }
}
