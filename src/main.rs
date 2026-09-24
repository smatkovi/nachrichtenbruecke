//! Bruecke -- ein Telepathy-Verbindungsmanager fuer WhatsApp und Signal.
//!
//! Er tritt neben pybridge, nicht an seine Stelle: Telegram und Matrix
//! laufen dort weiter, bis hier etwas steht, dem man sie anvertrauen mag.
//! Deshalb ein eigener Bus-Name.
//!
//! ## Warum ueberhaupt neu
//!
//! pybridge ist in Python geschrieben und spricht rohes D-Bus -- keine
//! Telepathy-Bibliothek, 1700 Zeilen. Das ist kein Vorwurf, es
//! funktioniert. Aber drei Dinge davon haben auf diesem Geraet Schaden
//! angerichtet, und alle drei sind hier anders geloest:
//!
//!  * **Kanaele melden.** pybridge startet dafuer einen eigenen
//!    Python-Prozess je Meldung. Beim Nachholen vieler Nachrichten trieb
//!    das die Last auf ueber sieben. Hier geschieht es im selben Prozess.
//!
//!  * **Die Chatliste.** pybridge holt sie genau einmal beim Verbinden.
//!    War der Dienst gerade neu gestartet und antwortete mit null Chats,
//!    blieb die Namenstabelle fuer die ganze Sitzung leer -- in der
//!    Nachrichten-App standen dann rohe Nummern, und wer darin antwortete,
//!    schrieb unbemerkt in einen Einzelchat statt in die Gruppe. Hier wird
//!    sie nachgeladen, solange sie leer ist.
//!
//!  * **Der Umweg ueber Python-Daemons.** pybridge spricht mit den
//!    Diensten ueber einen Unix-Socket und einen Uebersetzer dazwischen.
//!    Beide Dienste bieten dieselbe HTTP-Schnittstelle; der Uebersetzer
//!    entfaellt, und mit ihm zwei Prozesse.
//!
//! Dazu kommt der Platz: pybridge braucht 9 MB, telepathy-ring in C 2,8.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

mod meldung;
mod dienst;
mod hintergrund;
mod kanal;
mod kennungen;
mod kontenwache;
mod lauf;
mod socketdienst;
mod verbindung;

pub const CM_NAME: &str = "bruecke";
pub const CM_BUS: &str = "org.freedesktop.Telepathy.ConnectionManager.bruecke";
pub const CM_PFAD: &str = "/org/freedesktop/Telepathy/ConnectionManager/bruecke";

pub const IF_CM: &str = "org.freedesktop.Telepathy.ConnectionManager";
pub const IF_VERBINDUNG: &str = "org.freedesktop.Telepathy.Connection";
pub const IF_KANAL: &str = "org.freedesktop.Telepathy.Channel";
pub const IF_TEXT: &str = "org.freedesktop.Telepathy.Channel.Type.Text";
pub const IF_REQUESTS: &str =
    "org.freedesktop.Telepathy.Connection.Interface.Requests";
pub const IF_PRESENCE: &str =
    "org.freedesktop.Telepathy.Connection.Interface.SimplePresence";
pub const IF_CONTACTS: &str =
    "org.freedesktop.Telepathy.Connection.Interface.Contacts";
pub const IF_ALIASING: &str =
    "org.freedesktop.Telepathy.Connection.Interface.Aliasing";
/// Eigene Schnittstelle: das Antippen einer Meldung landet hier.
pub const IF_MELDUNG: &str = "org.smatkovi.Bruecke.Meldung";

pub const GRIFF_KONTAKT: u32 = 1;

pub const STATUS_VERBUNDEN: u32 = 0;
pub const STATUS_VERBINDET: u32 = 1;
pub const STATUS_GETRENNT: u32 = 2;

/// Die Protokolle, die dieser Manager traegt.
///
/// Zwei Wege zum Hintergrund: WhatsApp und Signal sprechen HTTP mit
/// unseren eigenen Diensten, Telegram und Matrix Zeilen-JSON ueber einen
/// Unix-Socket mit den vorhandenen Python-Daemons. Letztere bleiben, wie
/// sie sind – sie zu ersetzen waere ein zweites Projekt.
pub const PROTOKOLLE: [&str; 4] = ["whatsapp", "signal", "telegram", "matrix"];

pub fn protokoll_bekannt(p: &str) -> bool {
    PROTOKOLLE.contains(&p)
}

/// Der Verbindungsmanager.
pub struct Manager {
    /// Protokoll -> Verbindung (Bus-Name, Pfad).
    pub verbindungen: Arc<Mutex<HashMap<String, (String, String)>>>,
}

#[zbus::interface(name = "org.freedesktop.Telepathy.ConnectionManager")]
impl Manager {
    /// Welche Protokolle es gibt.
    ///
    /// Anders als bei pybridge haengt das nicht daran, ob eine Datei auf
    /// der Platte liegt: der Dienst darf gerade aus sein, das Protokoll
    /// gibt es trotzdem.
    async fn list_protocols(&self) -> Vec<String> {
        PROTOKOLLE.iter().map(|s| s.to_string()).collect()
    }

    /// Die Parameter, nach denen die Kontoverwaltung fragt.
    ///
    /// (Name, Flags, Signatur, Vorgabe) – Flag 4 heisst "erforderlich".
    async fn get_parameters(
        &self,
        protokoll: &str,
    ) -> Vec<(String, u32, String, zbus::zvariant::OwnedValue)> {
        if !protokoll_bekannt(protokoll) {
            return Vec::new();
        }
        let leer = zbus::zvariant::Value::from("").try_into().unwrap();
        vec![("account".to_string(), 4u32, "s".to_string(), leer)]
    }

    async fn request_connection(
        &self,
        #[zbus(connection)] bus: &zbus::Connection,
        protokoll: &str,
        _parameter: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<(String, zbus::zvariant::OwnedObjectPath)> {
        if !protokoll_bekannt(protokoll) {
            return Err(zbus::fdo::Error::NotSupported(format!(
                "kein solches Protokoll: {protokoll}"
            )));
        }
        let pfad = format!(
            "/org/freedesktop/Telepathy/Connection/{CM_NAME}/{protokoll}/{protokoll}"
        );
        let busname = format!(
            "org.freedesktop.Telepathy.Connection.{CM_NAME}.{protokoll}.{protokoll}"
        );

        // Schon einmal angefordert? Dann dieselbe zurueckgeben -- eine
        // zweite Verbindung desselben Kontos waere ein zweiter Draht zum
        // Dienst, und davon wird nichts besser.
        {
            let mut v = self.verbindungen.lock().await;
            if let Some((b, p)) = v.get(protokoll) {
                return Ok((
                    b.clone(),
                    zbus::zvariant::ObjectPath::try_from(p.clone())
                        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
                        .into(),
                ));
            }
            v.insert(protokoll.to_string(), (busname.clone(), pfad.clone()));
        }

        eprintln!("Verbindung angefordert: {protokoll}");

        let z: verbindung::GeteilterZustand =
            std::sync::Arc::new(Mutex::new(verbindung::Zustand::neu(protokoll)));
        let (auftrag_an, auftrag_von) = tokio::sync::mpsc::unbounded_channel();
        let (kanal_an, kanal_von) = tokio::sync::mpsc::unbounded_channel();

        // Alle vier Schnittstellen liegen unter derselben Adresse.
        let server = bus.object_server();
        server
            .at(pfad.clone(), verbindung::Verbindung {
                z: z.clone(),
                auftraege: auftrag_an.clone(),
            })
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let _ = server
            .at(pfad.clone(), verbindung::Anfragen {
                z: z.clone(),
                kanal_anlegen: kanal_an,
            })
            .await;
        let _ = server
            .at(pfad.clone(), verbindung::Anwesenheit {
                z: z.clone(),
                auftraege: auftrag_an.clone(),
            })
            .await;
        let _ = server
            .at(pfad.clone(), verbindung::Namen { z: z.clone() })
            .await;
        let _ = server
            .at(pfad.clone(), verbindung::Kontakte { z: z.clone() })
            .await;

        // Der eigene Bus-Name der Verbindung. Mission Control spricht sie
        // darunter an, nicht unter dem des Managers.
        let _ = bus
            .request_name_with_flags(
                busname.as_str(),
                zbus::fdo::RequestNameFlags::DoNotQueue.into(),
            )
            .await;

        // Der Meldungsdienst ist nicht lebenswichtig: fehlt er, laeuft
        // alles weiter, nur ohne sichtbaren Hinweis.
        let melder = meldung::Melder::neu(bus.clone()).await;
        let (gelesen_an, gelesen) = tokio::sync::mpsc::unbounded_channel();

        let lauf = lauf::Lauf {
            bus: bus.clone(),
            z: z.clone(),
            auftraege: auftrag_von,
            kanal_wuensche: kanal_von,
            senden_an: auftrag_an,
            kanalzustaende: HashMap::new(),
            melder,
            gelesen_an,
            gelesen,
        };
        tokio::spawn(lauf.laufen());

        Ok((
            busname,
            zbus::zvariant::ObjectPath::try_from(pfad)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
                .into(),
        ))
    }

    #[zbus(signal)]
    async fn new_connection(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        bus_name: &str,
        pfad: zbus::zvariant::ObjectPath<'_>,
        protokoll: &str,
    ) -> zbus::Result<()>;
}

/// Die Eigenschaften eines Kanals, wie Telepathy sie erwartet.
///
/// Sie stehen an mehreren Stellen gleich – beim Anlegen, beim Melden und
/// in der Channels-Eigenschaft. Einmal geschrieben statt dreimal.
pub fn kanal_eigenschaften(
    z: &verbindung::Zustand,
    griff: u32,
    _pfad: &str,
) -> HashMap<String, zbus::zvariant::OwnedValue> {
    use zbus::zvariant::Value;
    // Das Schild, nicht die Kennung: die Nachrichten-App zeigt TargetID
    // als Namen des Gespraechs an. Zum Adressieren taugt es trotzdem --
    // aufloesen() findet den Griff dazu zurueck, und der Kanal kennt
    // seine echte Kennung selbst.
    let kennung = z.kennungen.schild(griff);
    let mut m: HashMap<String, zbus::zvariant::OwnedValue> = HashMap::new();
    m.insert(format!("{IF_KANAL}.ChannelType"),
             Value::from(IF_TEXT).try_into().unwrap());
    m.insert(format!("{IF_KANAL}.TargetHandleType"),
             Value::from(GRIFF_KONTAKT).try_into().unwrap());
    m.insert(format!("{IF_KANAL}.TargetHandle"),
             Value::from(griff).try_into().unwrap());
    m.insert(format!("{IF_KANAL}.TargetID"),
             Value::from(kennung.clone()).try_into().unwrap());
    m.insert(format!("{IF_KANAL}.Requested"),
             Value::from(false).try_into().unwrap());
    m.insert(format!("{IF_KANAL}.InitiatorHandle"),
             Value::from(griff).try_into().unwrap());
    m.insert(format!("{IF_KANAL}.InitiatorID"),
             Value::from(kennung).try_into().unwrap());
    // Text ist die ART des Kanals, keine zusaetzliche Schnittstelle --
    // sie hier noch einmal zu nennen war falsch.
    m.insert(format!("{IF_KANAL}.Interfaces"),
             Value::from(Vec::<String>::new()).try_into().unwrap());
    m
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manager = Manager {
        verbindungen: Arc::new(Mutex::new(HashMap::new())),
    };

    let verbindung = zbus::connection::Builder::session()?
        .name(CM_BUS)?
        .serve_at(CM_PFAD, manager)?
        .build()
        .await?;

    // Das Protokoll geht auf die Fehlerausgabe, nicht auf die
    // Standardausgabe: letztere ist bei einer Umleitung blockgepuffert,
    // und dann steht die entscheidende Zeile noch im Puffer, waehrend man
    // im Protokoll nach ihr sucht.
    // Die Wache haelt die Konten oben, die oben sein sollen -- und nur
    // die. Ein Konto, das jemand bewusst abgeschaltet hat, bleibt aus.
    kontenwache::starten(verbindung.clone());

    eprintln!("🌉 {CM_BUS} bereit");
    let _ = verbindung;
    std::future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Kein Doppelbindestrich in einem Doc-Kommentar.
    ///
    /// zbus schreibt Doc-Kommentare als XML-Kommentare in die
    /// Introspektionsdaten, und dort ist "\-\-" verboten. Ein einziger
    /// Gedankenstrich in einem Doc-Kommentar macht das XML unlesbar;
    /// commhistory-daemon scheitert dann an becomeReady und antwortet auf
    /// ObserveChannels mit InvalidArgument ohne Text. Genau so ist der
    /// Verlauf der Nachrichten-App monatelang leer geblieben.
    #[test]
    fn doc_kommentare_ohne_doppelbindestrich() {
        let quellen = [
            include_str!("main.rs"),
            include_str!("verbindung.rs"),
            include_str!("kanal.rs"),
            include_str!("lauf.rs"),
            include_str!("kennungen.rs"),
            include_str!("hintergrund.rs"),
            include_str!("socketdienst.rs"),
            include_str!("dienst.rs"),
            include_str!("kontenwache.rs"),
        ];
        let mut schlimm = Vec::new();
        for (i, quelle) in quellen.iter().enumerate() {
            for (n, zeile) in quelle.lines().enumerate() {
                let z = zeile.trim_start();
                // Der Test enthaelt das Muster selbst; die eigene Zeile
                // ist maskiert und faellt darum nicht auf.
                if z.starts_with("///") && z.contains("--") {
                    schlimm.push(format!("Quelle {i}, Zeile {}: {z}", n + 1));
                }
            }
        }
        assert!(schlimm.is_empty(), "Doppelbindestrich im Doc-Kommentar:\n{}", schlimm.join("\n"));
    }
}
