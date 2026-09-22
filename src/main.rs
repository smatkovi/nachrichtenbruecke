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

mod dienst;
mod kanal;
mod kennungen;
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

pub const GRIFF_KONTAKT: u32 = 1;

pub const STATUS_VERBUNDEN: u32 = 0;
pub const STATUS_VERBINDET: u32 = 1;
pub const STATUS_GETRENNT: u32 = 2;

/// Die Protokolle, die dieser Manager traegt.
///
/// Zwei Wege zum Hintergrund: WhatsApp und Signal sprechen HTTP mit
/// unseren eigenen Diensten, Telegram und Matrix Zeilen-JSON ueber einen
/// Unix-Socket mit den vorhandenen Python-Daemons. Letztere bleiben, wie
/// sie sind -- sie zu ersetzen waere ein zweites Projekt.
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
    /// (Name, Flags, Signatur, Vorgabe) -- Flag 4 heisst "erforderlich".
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
        let bus = format!(
            "org.freedesktop.Telepathy.Connection.{CM_NAME}.{protokoll}.{protokoll}"
        );
        println!("Verbindung angefordert: {protokoll}");
        Ok((
            bus,
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
/// Sie stehen an mehreren Stellen gleich -- beim Anlegen, beim Melden und
/// in der Channels-Eigenschaft. Einmal geschrieben statt dreimal.
pub fn kanal_eigenschaften(
    z: &verbindung::Zustand,
    griff: u32,
    _pfad: &str,
) -> HashMap<String, zbus::zvariant::OwnedValue> {
    use zbus::zvariant::Value;
    let kennung = z.kennungen.name(griff);
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
    m.insert(format!("{IF_KANAL}.Interfaces"),
             Value::from(vec![IF_TEXT.to_string()]).try_into().unwrap());
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

    println!("🌉 {CM_BUS} bereit");
    let _ = verbindung;
    std::future::pending::<()>().await;
    Ok(())
}
