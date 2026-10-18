//! Die HTTP-Seite: der Draht zu den Diensten von WhatsApp und Signal.
//!
//! pybridge sprach mit beiden ueber einen Python-Daemon und einen
//! Unix-Socket. Der entfaellt hier: beide Dienste bieten dieselbe
//! HTTP-Schnittstelle, und wer sie ohnehin anspricht, braucht keinen
//! Uebersetzer dazwischen. Damit fallen zwei Python-Prozesse weg.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Anfrage hinaus, Antwort herein -- ueber jeden Strom, der beides kann.
///
/// Dieselben zwanzig Zeilen fuer TCP und Sockel: briard antwortet immer
/// mit `Connection: close` und laesst die Verbindung dann fallen, die
/// Dienste von WhatsApp und Signal ebenso. `read_to_end` bis zum Ende ist
/// deshalb der richtige Leser -- und der einzige, der auch mit
/// stueckweiser Uebertragung ohne Laengenangabe zurechtkommt.
async fn austausch<S>(mut strom: S, anfrage: &str) -> Result<String, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    strom
        .write_all(anfrage.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut roh = Vec::new();
    strom.read_to_end(&mut roh).await.map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&roh).into_owned())
}

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

/// Wo ein Dienst zu erreichen ist.
///
/// WhatsApp und Signal hoeren auf einem von wenigen Ports -- welcher es
/// ist, steht erst beim Suchen fest. Briar ist anders: briard hat mit
/// 0.42.0 den Port aufgegeben und liegt nur noch auf einem Unix-Sockel im
/// eigenen Datenordner, und zwar mit Anmeldung. Ein Port waere dort auch
/// nicht zu haben: `--api-port` wird ohne `BRIAR_API_TCP=1` stillschweigend
/// ignoriert, und ohne Geheimnis beantwortet briard selbst /status mit 401.
#[derive(Clone)]
enum Ziel {
    Ports { reihe: &'static [u16], gefunden: u16 },
    Sockel { pfad: PathBuf },
}

/// Ein Dienst, erreichbar ueber einen Port oder einen Sockel.
#[derive(Clone)]
pub struct Dienst {
    pub protokoll: String,
    ziel: Ziel,
}

/// Der Datenordner von harbour-briar -- dort liegen Sockel und Geheimnis.
pub fn briar_ordner() -> PathBuf {
    let heim = std::env::var("HOME").unwrap_or_else(|_| "/home/user".into());
    PathBuf::from(heim).join(".local/share/harbour-briar")
}

impl Dienst {
    pub fn neu(protokoll: &str) -> Option<Self> {
        let ziel = match protokoll {
            "whatsapp" => Ziel::Ports { reihe: &[8085, 8086, 8087, 8088, 8089], gefunden: 0 },
            "signal" => Ziel::Ports { reihe: &[8095, 8096, 8097, 8098, 8099], gefunden: 0 },
            "briar" => Ziel::Sockel { pfad: briar_ordner().join("api.sock") },
            _ => return None,
        };
        Some(Dienst { protokoll: protokoll.to_string(), ziel })
    }

    /// Sucht den Dienst. Der zuletzt gefundene Port wird zuerst probiert.
    pub async fn finden(&mut self) -> bool {
        match &self.ziel {
            Ziel::Sockel { .. } => self.hole_roh("/status", Duration::from_secs(3)).await.is_ok(),
            Ziel::Ports { reihe, gefunden } => {
                let mut folge: Vec<u16> = Vec::with_capacity(reihe.len() + 1);
                if *gefunden != 0 {
                    folge.push(*gefunden);
                }
                folge.extend_from_slice(reihe);
                for p in folge {
                    if let Ziel::Ports { gefunden, .. } = &mut self.ziel {
                        *gefunden = p;
                    }
                    if self.hole_roh("/status", Duration::from_secs(3)).await.is_ok() {
                        return true;
                    }
                }
                if let Ziel::Ports { gefunden, .. } = &mut self.ziel {
                    *gefunden = 0;
                }
                false
            }
        }
    }

    /// Wohin der Draht geht -- nur fuer das Protokoll.
    pub fn wo(&self) -> String {
        match &self.ziel {
            Ziel::Ports { gefunden, .. } => format!("Port {gefunden}"),
            Ziel::Sockel { pfad } => format!("Sockel {}", pfad.display()),
        }
    }

    /// Das Geheimnis fuer briard, frisch von der Platte.
    ///
    /// Nicht gemerkt: briard legt es bei jedem Start neu an (api.rs,
    /// `geheimnis_anlegen`), und wer es einmal einliest und behaelt, redet
    /// nach einem Neustart des Dienstes gegen lauter 401. 64 Byte aus dem
    /// eigenen Heimverzeichnis kosten weniger als dieser Fehler.
    fn geheimnis(&self) -> Option<String> {
        match &self.ziel {
            Ziel::Sockel { pfad } => {
                let datei = pfad.with_file_name("api-token");
                std::fs::read_to_string(datei)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            }
            Ziel::Ports { .. } => None,
        }
    }

    /// Ein sehr kleiner HTTP-Klient.
    ///
    /// Eine Kiste dafuer waere mehr Abhaengigkeit als Nutzen: es geht um
    /// GET auf 127.0.0.1 ohne Umleitungen, ohne TLS, ohne Anmeldung. Der
    /// Dienst antwortet mit Content-Length oder schliesst die Verbindung.
    async fn hole_roh(&self, pfad: &str, frist: Duration) -> Result<String, String> {
        let anmeldung = match self.geheimnis() {
            // Zwei Schreibweisen nimmt briard an; "Bearer" ist die, die
            // auch seine eigene Oberflaeche schickt (api.rs, `koepfe`).
            Some(g) => format!("Authorization: Bearer {g}\r\n"),
            None => String::new(),
        };
        let anfrage = format!(
            "GET {pfad} HTTP/1.1\r\nHost: 127.0.0.1\r\n{anmeldung}Connection: close\r\n\r\n"
        );
        let arbeit = async {
            let text = match &self.ziel {
                Ziel::Ports { gefunden, .. } => {
                    if *gefunden == 0 {
                        return Err("kein Port bekannt".into());
                    }
                    let strom = tokio::net::TcpStream::connect(format!("127.0.0.1:{gefunden}"))
                        .await
                        .map_err(|e| e.to_string())?;
                    austausch(strom, &anfrage).await?
                }
                Ziel::Sockel { pfad } => {
                    let strom = tokio::net::UnixStream::connect(pfad)
                        .await
                        .map_err(|e| format!("{}: {e}", pfad.display()))?;
                    austausch(strom, &anfrage).await?
                }
            };
            let (kopf, koerper) = text
                .split_once("\r\n\r\n")
                .ok_or_else(|| "Antwort ohne Kopf".to_string())?;
            let erste = kopf.lines().next().unwrap_or("");
            if !erste.contains(" 200 ") {
                // Der Grund steht im Koerper, nicht in der Statuszeile. Ohne
                // ihn stand im Protokoll nur "HTTP: HTTP/1.1 500 Internal
                // Server Error", und warum eine Nachricht nicht hinausging,
                // war daran nicht zu erkennen -- im Fall, der das ans Licht
                // brachte, war das Backend gerade von WhatsApp getrennt.
                let grund: String = koerper.trim().chars().take(200).collect();
                if grund.is_empty() {
                    return Err(format!("HTTP: {erste}"));
                }
                return Err(format!("HTTP: {erste} -- {grund}"));
            }
            // Stueckweise Uebertragung ist bei HTTP/1.1 nicht die
            // Ausnahme, sondern der Normalfall, wenn der Absender die
            // Laenge nicht vorher kennt. Das Go-Backend von WhatsApp
            // macht genau das; der Koerper begann dann mit "3895\r\n",
            // einer Chunk-Groesse in Hexadezimal, und der JSON-Leser
            // meldete "invalid type: integer". Der Dienst von Signal
            // schickt feste Laengen -- deshalb fiel es dort nie auf.
            let stueckweise = kopf
                .to_lowercase()
                .contains("transfer-encoding: chunked");
            if stueckweise {
                Ok(entstuecken(koerper))
            } else {
                Ok(koerper.to_string())
            }
        };
        match tokio::time::timeout(frist, arbeit).await {
            Ok(r) => r,
            Err(_) => Err("Zeitgrenze".into()),
        }
    }

    pub async fn chats(&self) -> Result<Vec<Chat>, String> {
        let roh = self.hole_roh("/chats", Duration::from_secs(20)).await?;
        serde_json::from_str(&roh).map_err(|e| {
            let anfang: String = roh.chars().take(120).collect();
            format!("{e} -- Anfang der Antwort: {anfang:?}")
        })
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

    /// Antwortet der Dienst noch?
    ///
    /// Fuer WhatsApp und Signal ist die eigene Nummer die Antwort: sie
    /// steht erst da, wenn das Backend wirklich angemeldet ist -- ein
    /// blankes 200 hiesse nur "Prozess laeuft", und darauf ist schon
    /// einmal eine stumme Verbindung gebaut worden.
    ///
    /// Briar hat keine Nummer, und `phone` kommt in briard nirgends vor.
    /// Bis hierher war `lebt()` fuer Briar also immer falsch, und
    /// lauf.rs warf die gerade aufgebaute Verbindung im naechsten
    /// Durchlauf wieder weg -- "Griff ist tot -- verbinde neu", im Kreis.
    /// Hier zaehlt stattdessen, dass /status ein Objekt mit `version`
    /// liefert: das steht in beiden Zweigen von briards `status()`, auch
    /// im zugesperrten, und kommt nur mit gueltigem Geheimnis heraus.
    pub async fn lebt(&self) -> bool {
        if self.protokoll != "briar" {
            return self.eigene_nummer().await.is_some();
        }
        let Ok(roh) = self.hole_roh("/status", Duration::from_secs(5)).await else {
            return false;
        };
        serde_json::from_str::<serde_json::Value>(&roh)
            .ok()
            .and_then(|v| v.get("version").cloned())
            .is_some()
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

/// Setzt einen stueckweise uebertragenen Koerper wieder zusammen.
///
/// Jedes Stueck beginnt mit seiner Laenge in Hexadezimal, gefolgt von
/// CRLF, dann die Daten, dann wieder CRLF. Ein Stueck der Laenge null
/// beendet den Koerper.
fn entstuecken(roh: &str) -> String {
    let mut aus = String::with_capacity(roh.len());
    let mut rest = roh;
    loop {
        let Some((kopf, danach)) = rest.split_once("\r\n") else { break };
        // Nach der Laenge koennen Erweiterungen stehen; sie gehen uns
        // nichts an.
        let laengentext = kopf.split(';').next().unwrap_or("").trim();
        let Ok(laenge) = usize::from_str_radix(laengentext, 16) else { break };
        if laenge == 0 {
            break;
        }
        if danach.len() < laenge {
            // Unvollstaendig -- lieber das Bekannte als gar nichts.
            aus.push_str(danach);
            break;
        }
        aus.push_str(&danach[..laenge]);
        rest = danach[laenge..].trim_start_matches("\r\n");
    }
    aus
}
