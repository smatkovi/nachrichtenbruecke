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
