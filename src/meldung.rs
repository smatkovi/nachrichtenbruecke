//! Die sichtbare Meldung ueber eine neue Nachricht.
//!
//! Eigentlich waere das nicht unsere Aufgabe: auf Harmattan meldet
//! commhistory-daemon neue Nachrichten, sobald ihm ein Kanal zugestellt
//! wird. Er nimmt aber keinen an – seit die Bruecke laeuft, in jedem
//! einzelnen Fall, mit InvalidArgument und ohne einen Satz dazu. Der Ton
//! kam trotzdem, das Banner nie.
//!
//! Also melden wir selbst. Denselben Weg geht der WhatsApp-Port fuer
//! seine Anrufe, und dort tut er seit Monaten, was er soll.

use std::collections::HashMap;

/// Der Ereignistyp; die zugehoerige .conf liegt unter
/// /usr/share/meegotouch/notifications/eventtypes/.
const TYP: &str = "bruecke.nachricht";

const DIENST: &str = "com.meego.core.MNotificationManager";
const PFAD: &str = "/notificationmanager";
const SCHNITTSTELLE: &str = "com.meego.core.MNotificationManager";

/// Was zu einem Chat gerade auf dem Bildschirm steht.
struct Stand {
    nummer: u32,
    ungelesen: u32,
}

pub struct Melder {
    bus: zbus::Connection,
    benutzer: u32,
    offen: HashMap<String, Stand>,
}

impl Melder {
    pub async fn neu(bus: zbus::Connection) -> Option<Self> {
        let antwort = bus
            .call_method(Some(DIENST), PFAD, Some(SCHNITTSTELLE), "notificationUserId", &())
            .await
            .ok()?;
        let benutzer: u32 = antwort.body().deserialize().ok()?;
        eprintln!("🔔 Meldungen unter Kennung {benutzer}");
        Some(Melder { bus, benutzer, offen: HashMap::new() })
    }

    /// Meldet eine neue Nachricht, oder zaehlt eine bestehende Meldung hoch.
    ///
    /// Je Chat eine Meldung, nicht je Nachricht: fuenf Zeilen in einem
    /// Gespraech sind ein Ereignis, keine fuenf.
    pub async fn melden(
        &mut self,
        kennung: &str,
        chat_name: &str,
        absender: &str,
        text: &str,
        kanalpfad: &str,
    ) {
        let titel = if chat_name.is_empty() { kennung } else { chat_name };
        // In einer Gruppe gehoert der Absender vor den Text; im Einzelchat
        // steht er schon in der Ueberschrift.
        let koerper = if absender.is_empty() {
            text.to_string()
        } else {
            format!("{absender}: {text}")
        };
        // Beim Antippen das Gespraech oeffnen. Die Aktion ist ein
        // D-Bus-Aufruf in Textform, vier Woerter: Dienst, Pfad,
        // Schnittstelle, Methode. Argumente waeren hier nicht verlaesslich
        // unterzubringen, deshalb steht der Chat im Pfad – der Kanal weiss,
        // wer er ist.
        let aktion = format!("{} {kanalpfad} {} Oeffnen", crate::CM_BUS, crate::IF_MELDUNG);
        let kennzeichen = format!("bruecke-{kennung}");

        let ungelesen = self.offen.get(kennung).map(|s| s.ungelesen + 1).unwrap_or(1);

        if let Some(stand) = self.offen.get(kennung) {
            let args = (
                self.benutzer,
                stand.nummer,
                TYP,
                titel,
                koerper.as_str(),
                aktion.as_str(),
                "",
                ungelesen,
                kennzeichen.as_str(),
            );
            if self
                .bus
                .call_method(Some(DIENST), PFAD, Some(SCHNITTSTELLE), "updateNotification", &args)
                .await
                .is_ok()
            {
                self.offen.insert(
                    kennung.to_string(),
                    Stand { nummer: stand.nummer, ungelesen },
                );
                return;
            }
            // Die alte Meldung gibt es nicht mehr – dann eine neue.
            self.offen.remove(kennung);
        }

        let args = (
            self.benutzer,
            0u32,
            TYP,
            titel,
            koerper.as_str(),
            aktion.as_str(),
            "",
            ungelesen,
            kennzeichen.as_str(),
        );
        match self
            .bus
            .call_method(Some(DIENST), PFAD, Some(SCHNITTSTELLE), "addNotification", &args)
            .await
        {
            Ok(antwort) => match antwort.body().deserialize::<u32>() {
                Ok(nummer) => {
                    self.offen.insert(kennung.to_string(), Stand { nummer, ungelesen: 1 });
                }
                Err(e) => eprintln!("🔔 Meldung ohne Nummer: {e}"),
            },
            Err(e) => eprintln!("🔔 Meldung nicht absetzbar: {e}"),
        }
    }

    /// Nimmt die Meldung weg, sobald der Chat gelesen ist.
    pub async fn schliessen(&mut self, kennung: &str) {
        let Some(stand) = self.offen.remove(kennung) else { return };
        let _ = self
            .bus
            .call_method(
                Some(DIENST),
                PFAD,
                Some(SCHNITTSTELLE),
                "removeNotification",
                &(self.benutzer, stand.nummer),
            )
            .await;
    }
}
