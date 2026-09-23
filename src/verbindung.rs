//! Eine Verbindung -- ein angemeldetes Konto eines Protokolls.
//!
//! Telepathy erwartet hier mehrere Schnittstellen unter derselben
//! Adresse: Connection, Requests, SimplePresence und Contacts. In zbus
//! sind das eigene Typen, die sich denselben Zustand teilen.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::kennungen::Kennungen;
use crate::{
    GRIFF_KONTAKT, IF_ALIASING, IF_CONTACTS, IF_KANAL, IF_PRESENCE, IF_REQUESTS,
    IF_TEXT, STATUS_GETRENNT, STATUS_VERBINDET, STATUS_VERBUNDEN,
};

pub struct Zustand {
    pub protokoll: String,
    pub pfad: String,
    pub status: u32,
    pub kennungen: Kennungen,
    /// Griff -> Kanalpfad.
    pub kanaele: HashMap<u32, String>,
    /// Griff -> Pfad, der einem Anfragenden schon genannt wurde.
    ///
    /// EnsureChannel muss den Pfad sofort zurueckgeben, angelegt wird der
    /// Kanal aber erst in der Hauptschleife. Ohne diese Vormerkung zaehlt
    /// dort die naechste Nummer noch einmal hoch, und der Anfragende haelt
    /// einen Pfad in der Hand, unter dem nie etwas erscheint.
    pub vorgemerkt: HashMap<u32, String>,
    pub naechster_kanal: u32,
    /// Chat-Kennung -> Anzeigename.
    pub chats: HashMap<String, String>,
    pub selbst: u32,
}

impl Zustand {
    pub fn neu(protokoll: &str) -> Self {
        Zustand {
            protokoll: protokoll.to_string(),
            pfad: format!(
                "/org/freedesktop/Telepathy/Connection/{}/{protokoll}/{protokoll}",
                crate::CM_NAME
            ),
            status: STATUS_GETRENNT,
            kennungen: Kennungen::laden(protokoll),
            kanaele: HashMap::new(),
            vorgemerkt: HashMap::new(),
            // Wie bei pybridge aus der Uhr, damit Pfade aus frueheren
            // Laeufen nicht wiederverwendet werden.
            naechster_kanal: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                % 10000) as u32,
            chats: HashMap::new(),
            selbst: 0,
        }
    }
}

pub type GeteilterZustand = Arc<Mutex<Zustand>>;

/// Was die Verbindung von aussen aufgetragen bekommt.
pub enum Auftrag {
    Verbinden,
    Trennen,
    Senden { an: String, text: String },
}

#[derive(Clone)]
pub struct Verbindung {
    pub z: GeteilterZustand,
    pub auftraege: tokio::sync::mpsc::UnboundedSender<Auftrag>,
}

#[zbus::interface(name = "org.freedesktop.Telepathy.Connection")]
impl Verbindung {
    async fn connect(&self) {
        let _ = self.auftraege.send(Auftrag::Verbinden);
    }

    /// Trennen beendet nichts.
    ///
    /// Mission Control trennt eine Verbindung bei jeder Gelegenheit –
    /// beim Bildschirmschlaf etwa. Die Verbindung zum Dienst dabei
    /// abzubauen hiesse, jede eingehende Nachricht zu verpassen, bis
    /// jemand die App oeffnet. Der Zustand wird gemeldet, der Draht
    /// bleibt.
    async fn disconnect(&self) {
        let _ = self.auftraege.send(Auftrag::Trennen);
    }

    async fn get_status(&self) -> u32 {
        self.z.lock().await.status
    }

    async fn get_interfaces(&self) -> Vec<String> {
        vec![
            IF_REQUESTS.to_string(),
            IF_PRESENCE.to_string(),
            IF_CONTACTS.to_string(),
            IF_ALIASING.to_string(),
        ]
    }

    async fn get_protocol(&self) -> String {
        self.z.lock().await.protokoll.clone()
    }

    async fn get_self_handle(&self) -> u32 {
        self.z.lock().await.selbst
    }

    /// Die KENNUNG zu einem Griff, nicht der Anzeigename.
    ///
    /// Das ist der Vertrag von Telepathy, und er hat einen Grund: was
    /// hier herauskommt, kommt spaeter in RequestHandles und als
    /// TargetID wieder herein. Gab man den Anzeigenamen zurueck, legte
    /// die Nachrichten-App beim Antworten einen Griff auf "Fabian
    /// Mistelberger" an, und der Dienst bekam einen Namen als
    /// Empfaengeradresse. Der Name gehoert in Aliasing, nicht hierher.
    async fn inspect_handles(&self, _art: u32, griffe: Vec<u32>) -> Vec<String> {
        let z = self.z.lock().await;
        griffe
            .iter()
            .map(|g| {
                z.kennungen
                    .kennung(*g)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| z.kennungen.name(*g))
            })
            .collect()
    }

    /// Griffe zu Kennungen – aber nur fuer Kontakte.
    ///
    /// Fuer Listen (Art 3: stored, publish, subscribe, deny) fragt der
    /// Kontoverwalter ebenfalls hier an. Frueher landeten die vier Namen
    /// im selben Zahlenraum wie die Kontakte und liessen sich danach als
    /// Gespraech oeffnen. Wir tragen keine Kontaktlisten, also ist die
    /// ehrliche Antwort ein Fehler.
    async fn request_handles(
        &self,
        art: u32,
        namen: Vec<String>,
    ) -> zbus::fdo::Result<Vec<u32>> {
        if art != GRIFF_KONTAKT {
            return Err(zbus::fdo::Error::NotSupported(format!(
                "nur Kontaktgriffe, nicht Art {art}"
            )));
        }
        let mut z = self.z.lock().await;
        let mut griffe = Vec::with_capacity(namen.len());
        for n in &namen {
            let (g, ueber_namen) = z.kennungen.aufloesen(n);
            if ueber_namen {
                eprintln!("Griff ueber Anzeigenamen gefunden: {n:?} -> {g}");
            }
            griffe.push(g);
        }
        let p = z.protokoll.clone();
        z.kennungen.sichern(&p);
        Ok(griffe)
    }

    async fn hold_handles(&self, _art: u32, _griffe: Vec<u32>) {}

    async fn release_handles(&self, _art: u32, _griffe: Vec<u32>) {}

    // Der Rust-Name weicht ab, der D-Bus-Name nicht: zbus erzeugt fuer
    // die Eigenschaft "Status" selbst ein status_changed, und zwei
    // gleichnamige Funktionen vertraegt ein Typ nicht.
    #[zbus(signal, name = "StatusChanged")]
    pub async fn tp_status_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        status: u32,
        grund: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_channel(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        pfad: zbus::zvariant::ObjectPath<'_>,
        art: &str,
        griffart: u32,
        griff: u32,
        unterdrueckt: bool,
    ) -> zbus::Result<()>;

    #[zbus(property, name = "Status")]
    async fn status(&self) -> u32 {
        self.z.lock().await.status
    }

    #[zbus(property, name = "Interfaces")]
    async fn interfaces(&self) -> Vec<String> {
        vec![
            IF_REQUESTS.to_string(),
            IF_PRESENCE.to_string(),
            IF_CONTACTS.to_string(),
            IF_ALIASING.to_string(),
        ]
    }

    #[zbus(property, name = "SelfHandle")]
    async fn self_handle(&self) -> u32 {
        self.z.lock().await.selbst
    }
}

/// Die Schnittstelle, ueber die Kanaele angefordert werden.
#[derive(Clone)]
pub struct Anfragen {
    pub z: GeteilterZustand,
    /// Bittet die Hauptschleife, einen Kanal anzulegen und zu melden.
    pub kanal_anlegen: tokio::sync::mpsc::UnboundedSender<(u32, bool)>,
}

#[zbus::interface(name = "org.freedesktop.Telepathy.Connection.Interface.Requests")]
impl Anfragen {
    async fn create_channel(
        &self,
        anfrage: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<(zbus::zvariant::OwnedObjectPath, HashMap<String, zbus::zvariant::OwnedValue>)>
    {
        let (_, pfad, eigenschaften) = self.ensure_channel(anfrage).await?;
        Ok((pfad, eigenschaften))
    }

    async fn ensure_channel(
        &self,
        anfrage: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<(
        bool,
        zbus::zvariant::OwnedObjectPath,
        HashMap<String, zbus::zvariant::OwnedValue>,
    )> {
        // Wir tragen genau eine Art Kanal. Ohne diese Pruefung wird aus
        // EnsureChannel(ContactList, Art 3, "subscribe") ein Textkanal auf
        // die Zeichenkette "subscribe" -- derselbe Weg, auf dem die vier
        // Listennamen als Gespraechsfaeden in der Griffliste landeten.
        let art = anfrage
            .get(&format!("{IF_KANAL}.ChannelType"))
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default();
        if !art.is_empty() && art != IF_TEXT {
            return Err(zbus::fdo::Error::NotSupported(format!(
                "nur Textkanaele, nicht {art}"
            )));
        }
        let griffart = anfrage
            .get(&format!("{IF_KANAL}.TargetHandleType"))
            .and_then(|v| u32::try_from(v.clone()).ok())
            .unwrap_or(GRIFF_KONTAKT);
        if griffart != GRIFF_KONTAKT {
            return Err(zbus::fdo::Error::NotSupported(format!(
                "nur Kontaktgriffe, nicht Art {griffart}"
            )));
        }

        let griff = anfrage
            .get(&format!("{IF_KANAL}.TargetHandle"))
            .and_then(|v| u32::try_from(v.clone()).ok())
            .unwrap_or(0);
        let kennung = anfrage
            .get(&format!("{IF_KANAL}.TargetID"))
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default();

        let mut z = self.z.lock().await;
        let griff = if griff != 0 {
            // Ein Griff, den wir nicht kennen, gehoert zu einem alten Lauf
            // der Nachrichten-App. Frueher wurde daraus ein Kanal mit
            // leerer Kennung, und der Dienst bekam die leere Zeichenkette
            // als Empfaenger.
            if z.kennungen.kennung(griff).is_none() {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "unbekannter Griff {griff}"
                )));
            }
            griff
        } else if !kennung.is_empty() {
            let (g, ueber_namen) = z.kennungen.aufloesen(&kennung);
            if ueber_namen {
                eprintln!("Kanal ueber Anzeigenamen gefunden: {kennung:?} -> {g}");
            }
            let p = z.protokoll.clone();
            z.kennungen.sichern(&p);
            g
        } else {
            return Err(zbus::fdo::Error::InvalidArgs("weder Griff noch Kennung".into()));
        };
        let gab_es = z.kanaele.contains_key(&griff);
        let pfad = match z.kanaele.get(&griff).or_else(|| z.vorgemerkt.get(&griff)) {
            Some(p) => p.clone(),
            None => {
                z.naechster_kanal += 1;
                let p = format!("{}/TextChannel{}", z.pfad, z.naechster_kanal);
                z.vorgemerkt.insert(griff, p.clone());
                p
            }
        };
        let eigenschaften = crate::kanal_eigenschaften(&z, griff, &pfad);
        drop(z);

        // Anlegen und melden geschieht in der Hauptschleife: von hier aus
        // darf nicht auf den Objektserver zugegriffen werden, waehrend
        // dieser Aufruf noch laeuft.
        let _ = self.kanal_anlegen.send((griff, true));
        Ok((
            gab_es,
            zbus::zvariant::ObjectPath::try_from(pfad)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
                .into(),
            eigenschaften,
        ))
    }

    #[zbus(signal)]
    pub async fn new_channels(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        kanaele: Vec<(
            zbus::zvariant::ObjectPath<'_>,
            HashMap<String, zbus::zvariant::OwnedValue>,
        )>,
    ) -> zbus::Result<()>;

    #[zbus(property, name = "Channels")]
    async fn channels(
        &self,
    ) -> Vec<(
        zbus::zvariant::OwnedObjectPath,
        HashMap<String, zbus::zvariant::OwnedValue>,
    )> {
        let z = self.z.lock().await;
        let mut aus = Vec::new();
        for (griff, pfad) in &z.kanaele {
            if let Ok(p) = zbus::zvariant::ObjectPath::try_from(pfad.clone()) {
                aus.push((p.into(), crate::kanal_eigenschaften(&z, *griff, pfad)));
            }
        }
        aus
    }

    #[zbus(property, name = "RequestableChannelClasses")]
    async fn requestable_channel_classes(
        &self,
    ) -> Vec<(HashMap<String, zbus::zvariant::OwnedValue>, Vec<String>)> {
        let mut feste: HashMap<String, zbus::zvariant::OwnedValue> = HashMap::new();
        feste.insert(
            format!("{IF_KANAL}.ChannelType"),
            zbus::zvariant::Value::from(IF_TEXT).try_into().unwrap(),
        );
        feste.insert(
            format!("{IF_KANAL}.TargetHandleType"),
            zbus::zvariant::Value::from(GRIFF_KONTAKT).try_into().unwrap(),
        );
        vec![(
            feste,
            vec![
                format!("{IF_KANAL}.TargetHandle"),
                format!("{IF_KANAL}.TargetID"),
            ],
        )]
    }
}

/// Anwesenheit – hier nur so weit, wie die Kontoverwaltung sie braucht.
#[derive(Clone)]
pub struct Anwesenheit {
    pub z: GeteilterZustand,
    pub auftraege: tokio::sync::mpsc::UnboundedSender<Auftrag>,
}

#[zbus::interface(
    name = "org.freedesktop.Telepathy.Connection.Interface.SimplePresence"
)]
impl Anwesenheit {
    async fn set_presence(&self, status: &str, _meldung: &str) {
        if status == "offline" {
            let _ = self.auftraege.send(Auftrag::Trennen);
        } else {
            let _ = self.auftraege.send(Auftrag::Verbinden);
        }
    }

    async fn get_presences(&self, griffe: Vec<u32>) -> HashMap<u32, (u32, String, String)> {
        let z = self.z.lock().await;
        let an = z.status == STATUS_VERBUNDEN;
        griffe
            .iter()
            .map(|g| {
                (
                    *g,
                    if an {
                        (2u32, "available".to_string(), String::new())
                    } else {
                        (1u32, "offline".to_string(), String::new())
                    },
                )
            })
            .collect()
    }

    #[zbus(signal)]
    pub async fn presences_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        anwesenheiten: HashMap<u32, (u32, String, String)>,
    ) -> zbus::Result<()>;

    #[zbus(property, name = "Statuses")]
    async fn statuses(
        &self,
    ) -> HashMap<String, (u32, bool, bool, HashMap<String, String>)> {
        let mut m = HashMap::new();
        m.insert("offline".to_string(), (1u32, true, false, HashMap::new()));
        m.insert("available".to_string(), (2u32, true, true, HashMap::new()));
        m
    }
}

/// Kontaktangaben, so weit CommHistory sie erwartet.
#[derive(Clone)]
pub struct Kontakte {
    pub z: GeteilterZustand,
}

#[zbus::interface(name = "org.freedesktop.Telepathy.Connection.Interface.Contacts")]
impl Kontakte {
    async fn get_contact_attributes(
        &self,
        griffe: Vec<u32>,
        _schnittstellen: Vec<String>,
        _halten: bool,
    ) -> HashMap<u32, HashMap<String, zbus::zvariant::OwnedValue>> {
        let z = self.z.lock().await;
        let mut aus = HashMap::new();
        for g in griffe {
            let mut m: HashMap<String, zbus::zvariant::OwnedValue> = HashMap::new();
            let name = z.kennungen.name(g);
            let kennung = z
                .kennungen
                .kennung(g)
                .map(|s| s.to_string())
                .unwrap_or_else(|| name.clone());
            m.insert(
                format!("{}/contact-id", crate::IF_VERBINDUNG),
                zbus::zvariant::Value::from(kennung).try_into().unwrap(),
            );
            m.insert(
                format!("{IF_ALIASING}/alias"),
                zbus::zvariant::Value::from(name.clone()).try_into().unwrap(),
            );
            m.insert(
                format!("{IF_PRESENCE}/presence"),
                zbus::zvariant::Value::from((2u32, "available".to_string(), String::new()))
                    .try_into()
                    .unwrap(),
            );
            aus.insert(g, m);
        }
        aus
    }

    #[zbus(property, name = "ContactAttributeInterfaces")]
    async fn contact_attribute_interfaces(&self) -> Vec<String> {
        vec![IF_PRESENCE.to_string(), IF_ALIASING.to_string()]
    }
}

/// Die Anzeigenamen.
///
/// Seit InspectHandles die Kennung liefert, ist das der einzige Weg, auf
/// dem die Nachrichten-App noch "Fabian Mistelberger" statt einer UUID
/// erfaehrt. Schreiben laesst sich nichts: die Namen kommen vom Dienst.
#[derive(Clone)]
pub struct Namen {
    pub z: GeteilterZustand,
}

#[zbus::interface(name = "org.freedesktop.Telepathy.Connection.Interface.Aliasing")]
impl Namen {
    async fn get_alias_flags(&self) -> u32 {
        0
    }

    async fn get_aliases(&self, griffe: Vec<u32>) -> HashMap<u32, String> {
        let z = self.z.lock().await;
        griffe.iter().map(|g| (*g, z.kennungen.name(*g))).collect()
    }

    async fn request_aliases(&self, griffe: Vec<u32>) -> Vec<String> {
        let z = self.z.lock().await;
        griffe.iter().map(|g| z.kennungen.name(*g)).collect()
    }

    /// Fremde Namen aendern wir nicht.
    async fn set_aliases(&self, _namen: HashMap<u32, String>) {}

    #[zbus(signal)]
    pub async fn aliases_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        namen: Vec<(u32, String)>,
    ) -> zbus::Result<()>;
}

pub const _S: (u32, u32, u32) = (STATUS_VERBUNDEN, STATUS_VERBINDET, STATUS_GETRENNT);
