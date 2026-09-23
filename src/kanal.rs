//! Ein Textkanal -- ein Gespraechsfaden, wie Telepathy ihn kennt.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{IF_KANAL, IF_TEXT};

/// Eine ausstehende Nachricht: (Nummer, Zeit, Absender, Art, Flags, Text).
pub type Ausstehend = (u32, u32, u32, u32, u32, String);

pub struct KanalZustand {
    pub pfad: String,
    pub griff: u32,
    pub griffart: u32,
    pub kennung: String,
    pub ausstehend: Vec<Ausstehend>,
    pub naechste_nummer: u32,
    pub geschlossen: bool,
    /// Wann zuletzt an CommHistory gemeldet – siehe Verbindung.
    pub zuletzt_gemeldet: std::time::Instant,
}

#[derive(Clone)]
pub struct Kanal {
    pub z: Arc<Mutex<KanalZustand>>,
}

#[zbus::interface(name = "org.freedesktop.Telepathy.Channel")]
impl Kanal {
    /// Schliessen wird bewusst ueberhoert.
    ///
    /// Die Nachrichten-App schliesst einen Kanal, sobald man die
    /// Unterhaltung verlaesst. Gaebe man ihn dann frei, ginge mit ihm der
    /// Gespraechsfaden verloren und die naechste Nachricht begaenne einen
    /// neuen. pybridge macht es ebenso, und aus demselben Grund.
    async fn close(&self) {}

    #[zbus(signal)]
    pub async fn closed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    async fn get_channel_type(&self) -> String {
        IF_TEXT.to_string()
    }

    async fn get_handle(&self) -> (u32, u32) {
        let z = self.z.lock().await;
        (z.griffart, z.griff)
    }

    /// Text ist die Art des Kanals, keine zusaetzliche Schnittstelle.
    async fn get_interfaces(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property, name = "ChannelType")]
    async fn channel_type(&self) -> String {
        IF_TEXT.to_string()
    }

    #[zbus(property, name = "TargetHandle")]
    async fn target_handle(&self) -> u32 {
        self.z.lock().await.griff
    }

    #[zbus(property, name = "TargetHandleType")]
    async fn target_handle_type(&self) -> u32 {
        self.z.lock().await.griffart
    }

    #[zbus(property, name = "TargetID")]
    async fn target_id(&self) -> String {
        self.z.lock().await.kennung.clone()
    }

    #[zbus(property, name = "Requested")]
    async fn requested(&self) -> bool {
        false
    }

    #[zbus(property, name = "InitiatorHandle")]
    async fn initiator_handle(&self) -> u32 {
        self.z.lock().await.griff
    }

    #[zbus(property, name = "InitiatorID")]
    async fn initiator_id(&self) -> String {
        self.z.lock().await.kennung.clone()
    }

    #[zbus(property, name = "Interfaces")]
    async fn interfaces(&self) -> Vec<String> {
        vec![IF_TEXT.to_string()]
    }
}

/// Der Textteil desselben Kanals – dieselbe Adresse, andere Schnittstelle.
#[derive(Clone)]
pub struct Text {
    pub z: Arc<Mutex<KanalZustand>>,
    pub senden: tokio::sync::mpsc::UnboundedSender<(u32, String, String)>,
}

#[zbus::interface(name = "org.freedesktop.Telepathy.Channel.Type.Text")]
impl Text {
    async fn send(&self, _art: u32, text: &str) {
        let (griff, kennung) = {
            let z = self.z.lock().await;
            (z.griff, z.kennung.clone())
        };
        // Nicht hier auf den Dienst warten: das Senden kann Sekunden
        // dauern, und solange stuende der ganze D-Bus still. Bei pybridge
        // war genau das der Grund fuer einen eigenen Faden je Sendung.
        let _ = self.senden.send((griff, kennung, text.to_string()));
    }

    /// Welche Arten von Nachrichten dieser Kanal traegt.
    ///
    /// Ohne diese Methode wird ein Textkanal in telepathy-qt nie fertig:
    /// GetMessageTypes gehoert zu den Aufrufen, die becomeReady macht.
    /// Scheitert einer davon, meldet commhistory-daemon auf
    /// ObserveChannels InvalidArgument ohne weiteren Text, und der
    /// Verlauf der Nachrichten-App bleibt leer.
    ///
    /// 0 ist Channel_Text_Message_Type_Normal. Mehr kennt die Bruecke
    /// nicht: Aktionen und Hinweise reicht sie als gewoehnlichen Text
    /// durch.
    async fn get_message_types(&self) -> Vec<u32> {
        vec![0]
    }

    async fn list_pending_messages(&self, loeschen: bool) -> Vec<Ausstehend> {
        let mut z = self.z.lock().await;
        let liste = z.ausstehend.clone();
        if loeschen {
            z.ausstehend.clear();
        }
        liste
    }

    async fn acknowledge_pending_messages(&self, nummern: Vec<u32>) {
        let mut z = self.z.lock().await;
        z.ausstehend.retain(|m| !nummern.contains(&m.0));
    }

    #[zbus(signal)]
    pub async fn received(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        nummer: u32,
        zeit: u32,
        absender: u32,
        art: u32,
        flags: u32,
        text: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn sent(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        zeit: u32,
        art: u32,
        text: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn send_error(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        fehler: u32,
        zeit: u32,
        art: u32,
        text: &str,
    ) -> zbus::Result<()>;
}

/// Damit der Kanal-Teil auch die Channel-Schnittstelle kennt.
pub const _IF: &str = IF_KANAL;
