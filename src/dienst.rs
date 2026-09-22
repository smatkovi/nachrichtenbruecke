//! Die HTTP-Seite: der Draht zu den Diensten von WhatsApp und Signal.
//!
//! pybridge sprach mit beiden ueber einen Python-Daemon und einen
//! Unix-Socket. Der entfaellt hier: beide Dienste bieten dieselbe
//! HTTP-Schnittstelle, und wer sie ohnehin anspricht, braucht keinen
//! Uebersetzer dazwischen. Damit fallen zwei Python-Prozesse weg.

use std::time::Duration;

use serde::Deserialize;

/// Was ein Dienst ueber einen Chat sagt.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Chat {
    pub jid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "isGroup")]
    pub is_group: bool,
    #[serde(default, rename = "lastMessage")]
    pub last_message: String,
    #[serde(default, rename = "lastTime")]
    pub last_time: u64,
    #[serde(default, rename = "fromMe")]
    pub from_me: bool,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Nachricht {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "chatJid")]
    pub chat_jid: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, rename = "fromMe")]
    pub from_me: bool,
    #[serde(default)]
    pub timestamp: u64,
    #[serde(default, rename = "mediaType")]
    pub media_type: String,
    #[serde(default, rename = "fileName")]
    pub file_name: String,
}

#[derive(Deserialize, Default)]
struct Ereignis {
    #[serde(default)]
    seq: i64,
}

/// Ein Dienst, erreichbar auf einem von wenigen Ports.
#[derive(Clone)]
pub struct Dienst {
    pub protokoll: &'static str,
    ports: &'static [u16],
    port: u16,
}

impl Dienst {
    pub fn neu(protokoll: &'static str) -> Option<Self> {
        let ports: &'static [u16] = match protokoll {
            "whatsapp" => &[8085, 8086, 8087, 8088, 8089],
            "signal" => &[8095, 8096, 8097, 8098, 8099],
            _ => return None,
        };
        Some(Dienst { protokoll, ports, port: 0 })
    }

    /// Sucht den Dienst. Der zuletzt gefundene Port wird zuerst probiert.
    pub async fn finden(&mut self) -> bool {
        let mut reihe: Vec<u16> = Vec::with_capacity(self.ports.len() + 1);
        if self.port != 0 {
            reihe.push(self.port);
        }
        reihe.extend_from_slice(self.ports);
        for p in reihe {
            self.port = p;
            if self.hole_roh("/status", Duration::from_secs(3)).await.is_ok() {
                return true;
            }
        }
        self.port = 0;
        false
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Ein sehr kleiner HTTP-Klient.
    ///
    /// Eine Kiste dafuer waere mehr Abhaengigkeit als Nutzen: es geht um
    /// GET auf 127.0.0.1 ohne Umleitungen, ohne TLS, ohne Anmeldung. Der
    /// Dienst antwortet mit Content-Length oder schliesst die Verbindung.
    async fn hole_roh(&self, pfad: &str, frist: Duration) -> Result<String, String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        if self.port == 0 {
            return Err("kein Port bekannt".into());
        }
        let adresse = format!("127.0.0.1:{}", self.port);
        let arbeit = async {
            let mut strom = tokio::net::TcpStream::connect(&adresse)
                .await
                .map_err(|e| e.to_string())?;
            let anfrage = format!(
                "GET {pfad} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            );
            strom
                .write_all(anfrage.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            let mut roh = Vec::new();
            strom.read_to_end(&mut roh).await.map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&roh).into_owned();
            let (kopf, koerper) = text
                .split_once("\r\n\r\n")
                .ok_or_else(|| "Antwort ohne Kopf".to_string())?;
            let erste = kopf.lines().next().unwrap_or("");
            if !erste.contains(" 200 ") {
                return Err(format!("HTTP: {erste}"));
            }
            Ok(koerper.to_string())
        };
        match tokio::time::timeout(frist, arbeit).await {
            Ok(r) => r,
            Err(_) => Err("Zeitgrenze".into()),
        }
    }

    pub async fn chats(&self) -> Result<Vec<Chat>, String> {
        let roh = self.hole_roh("/chats", Duration::from_secs(20)).await?;
        serde_json::from_str(&roh).map_err(|e| e.to_string())
    }

    pub async fn nachrichten(&self, jid: &str) -> Result<Vec<Nachricht>, String> {
        let pfad = format!("/messages?jid={}", kodieren(jid));
        let roh = self.hole_roh(&pfad, Duration::from_secs(20)).await?;
        serde_json::from_str(&roh).map_err(|e| e.to_string())
    }

    pub async fn senden(&self, an: &str, text: &str) -> Result<(), String> {
        let pfad = format!("/send?to={}&text={}", kodieren(an), kodieren(text));
        let roh = self.hole_roh(&pfad, Duration::from_secs(45)).await?;
        // Das WhatsApp-Backend antwortet mit dem blanken Wort "ok", das
        // von Signal mit JSON. Beides gilt.
        let t = roh.trim();
        if t == "ok" || t.contains("\"ok\"") {
            Ok(())
        } else {
            Err(t.to_string())
        }
    }

    /// Die lange Abfrage: kehrt zurueck, sobald sich etwas getan hat.
    pub async fn ereignis(&self, seit: i64) -> Result<i64, String> {
        let pfad = format!("/events?since={seit}");
        let roh = self.hole_roh(&pfad, Duration::from_secs(40)).await?;
        let e: Ereignis = serde_json::from_str(&roh).map_err(|e| e.to_string())?;
        Ok(e.seq)
    }

    pub async fn eigene_nummer(&self) -> Option<String> {
        let roh = self.hole_roh("/status", Duration::from_secs(5)).await.ok()?;
        let v: serde_json::Value = serde_json::from_str(&roh).ok()?;
        v.get("phone")?.as_str().map(|s| s.to_string())
    }
}

/// Prozentkodierung fuer einen Abfrageparameter.
///
/// Von Hand, und zwar sorgfaeltig: beim WhatsApp-Port kam durch doppelte
/// Kodierung einmal "%2C" statt eines Kommas beim Empfaenger an.
fn kodieren(s: &str) -> String {
    let mut aus = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                aus.push(*b as char)
            }
            _ => aus.push_str(&format!("%{b:02X}")),
        }
    }
    aus
}
