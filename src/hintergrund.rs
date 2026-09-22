//! Der Hintergrund eines Protokolls -- auf beiden Wegen dasselbe Bild.
//!
//! Nach aussen gibt es nur: verbinden, Chatliste, senden, und ein Strom
//! neuer Nachrichten. Ob dahinter HTTP oder ein Unix-Socket steckt, geht
//! den Rest der Bruecke nichts an.

use std::collections::{HashMap, HashSet};

use serde_json::json;
use tokio::sync::mpsc;

use crate::dienst::Dienst;
use crate::socketdienst::{self, SocketDienst};

/// Eine Nachricht, wie die Bruecke sie weiterreicht.
pub struct NeueNachricht {
    pub chat: String,
    pub chat_name: String,
    /// Anzeigename des Absenders; leer im Einzelchat.
    pub absender: String,
    pub text: String,
    pub zeit: u64,
}

pub enum Hintergrund {
    Http {
        dienst: Dienst,
        /// Zuletzt gesehener Stand je Chat.
        stand: HashMap<String, u64>,
        /// Schon ausgelieferte Nachrichten.
        gesehen: HashSet<String>,
        folge: i64,
        eingerichtet: bool,
    },
    Socket {
        dienst: SocketDienst,
        ereignisse: mpsc::UnboundedReceiver<serde_json::Value>,
    },
}

impl Hintergrund {
    pub async fn oeffnen(protokoll: &str) -> Result<Self, String> {
        match protokoll {
            "whatsapp" | "signal" => {
                let mut d = Dienst::neu(protokoll).ok_or("unbekannt")?;
                if !d.finden().await {
                    return Err(format!("{protokoll}: Dienst antwortet nicht"));
                }
                println!("{protokoll}: Dienst auf Port {}", d.port());
                Ok(Hintergrund::Http {
                    dienst: d,
                    stand: HashMap::new(),
                    gesehen: HashSet::new(),
                    folge: -1,
                    eingerichtet: false,
                })
            }
            "telegram" | "matrix" => {
                let (an, von) = mpsc::unbounded_channel();
                let d = SocketDienst::verbinden(protokoll, an).await?;
                // Die Kontakte kommen beim Telegram-Daemon nur auf
                // Nachfrage.
                let _ = d.fragen("get_me", serde_json::Value::Null).await;
                Ok(Hintergrund::Socket { dienst: d, ereignisse: von })
            }
            _ => Err("unbekanntes Protokoll".into()),
        }
    }

    /// Die Chatliste als Kennung -> Name.
    ///
    /// Eine leere Antwort wird NICHT als Ergebnis hingenommen. pybridge
    /// fragt genau einmal beim Verbinden, und wenn der Dienst dabei
    /// gerade hochfaehrt und nichts weiss, bleibt die Namenstabelle die
    /// ganze Sitzung leer -- in der Nachrichten-App stehen dann rohe
    /// Nummern statt Namen. Hier wird gewartet.
    pub async fn chats(&self) -> HashMap<String, String> {
        for versuch in 0..12 {
            let m = self.chats_einmal().await;
            if !m.is_empty() {
                if versuch > 0 {
                    println!("Chatliste kam erst nach {} s", versuch * 2);
                }
                return m;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        HashMap::new()
    }

    async fn chats_einmal(&self) -> HashMap<String, String> {
        match self {
            Hintergrund::Http { dienst, .. } => match dienst.chats().await {
                Ok(liste) => liste
                    .into_iter()
                    .map(|c| {
                        let name = if c.name.is_empty() { c.jid.clone() } else { c.name };
                        (c.jid, name)
                    })
                    .collect(),
                Err(_) => HashMap::new(),
            },
            Hintergrund::Socket { dienst, .. } => {
                let befehl = if dienst.protokoll() == "matrix" {
                    "get_rooms"
                } else {
                    "get_dialogs"
                };
                match dienst.fragen(befehl, json!({"limit": 200})).await {
                    Ok(v) => socketdienst::dialoge_lesen(dienst.protokoll(), &v)
                        .into_iter()
                        .map(|(k, t)| {
                            let name = if t.is_empty() { k.clone() } else { t };
                            (k, name)
                        })
                        .collect(),
                    Err(_) => HashMap::new(),
                }
            }
        }
    }

    /// Die eigene Kennung.
    ///
    /// Telepathy verlangt fuer eine verbundene Verbindung einen gueltigen
    /// eigenen Griff. Bleibt er null, gilt sie dem Kontoverwalter als
    /// nicht verbunden -- die Verbindung antwortet dann zwar auf alles,
    /// steht in der Oberflaeche aber auf "offline".
    pub async fn eigene_kennung(&self) -> String {
        match self {
            Hintergrund::Http { dienst, .. } => {
                dienst.eigene_nummer().await.unwrap_or_default()
            }
            Hintergrund::Socket { dienst, .. } => {
                match dienst.fragen("get_me", serde_json::Value::Null).await {
                    Ok(v) => v
                        .get("id")
                        .or_else(|| v.get("user_id"))
                        .map(socketdienst::wert_als_text)
                        .unwrap_or_default(),
                    Err(_) => String::new(),
                }
            }
        }
    }

    pub async fn senden(&self, an: &str, text: &str) -> Result<(), String> {
        match self {
            Hintergrund::Http { dienst, .. } => dienst.senden(an, text).await,
            Hintergrund::Socket { dienst, .. } => {
                let args = if dienst.protokoll() == "matrix" {
                    json!({"room_id": an, "body": text})
                } else {
                    // Telegram-Kennungen sind Zahlen; als Zeichenkette
                    // nimmt der Daemon sie nicht an.
                    match an.parse::<i64>() {
                        Ok(n) => json!({"chat_id": n, "text": text}),
                        Err(_) => json!({"chat_id": an, "text": text}),
                    }
                };
                let v = dienst.fragen("send_message", args).await?;
                if v.get("error").is_some() {
                    Err(v["error"].to_string())
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Wartet auf das Naechste und gibt zurueck, was hereinkam.
    pub async fn naechste(&mut self, namen: &HashMap<String, String>) -> Vec<NeueNachricht> {
        match self {
            Hintergrund::Socket { dienst, ereignisse } => {
                let Some(v) = ereignisse.recv().await else {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    return Vec::new();
                };
                let protokoll = dienst.protokoll().to_string();
                let Some(m) = socketdienst::nachricht_aus_ereignis(&protokoll, &v) else {
                    return Vec::new();
                };
                let chat = m.get("chat").cloned().unwrap_or_default();
                let absender_kennung = m.get("sender").cloned().unwrap_or_default();
                // In einer Gruppe gehoert der Absender ueber die
                // Nachricht; im Einzelchat waere es nur Laerm.
                let absender = if absender_kennung == chat || absender_kennung.is_empty()
                {
                    String::new()
                } else {
                    let n = m.get("sender_name").cloned().unwrap_or_default();
                    if n.is_empty() {
                        namen.get(&absender_kennung).cloned().unwrap_or(absender_kennung)
                    } else {
                        n
                    }
                };
                vec![NeueNachricht {
                    chat_name: m.get("chat_name").cloned().unwrap_or_default(),
                    chat,
                    absender,
                    text: m.get("text").cloned().unwrap_or_default(),
                    zeit: m
                        .get("zeit")
                        .and_then(|z| z.parse::<u64>().ok())
                        .map(|z| if z > 9_999_999_999 { z / 1000 } else { z })
                        .unwrap_or(0),
                }]
            }
            Hintergrund::Http { .. } => self.http_naechste(namen).await,
        }
    }

    async fn http_naechste(&mut self, namen: &HashMap<String, String>) -> Vec<NeueNachricht> {
        let Hintergrund::Http { dienst, stand, gesehen, folge, eingerichtet } = self else {
            return Vec::new();
        };

        // Lange Abfrage: kehrt zurueck, sobald sich im Dienst etwas tut.
        match dienst.ereignis(*folge).await {
            Ok(neu) => {
                if neu == *folge {
                    return Vec::new();
                }
                *folge = neu;
            }
            Err(_) => {
                // Dienst weg: in Ruhe warten statt zu haemmern.
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                if !dienst.finden().await {
                    return Vec::new();
                }
                return Vec::new();
            }
        }

        let Ok(chats) = dienst.chats().await else {
            return Vec::new();
        };

        if !*eingerichtet {
            // Beim ersten Lauf nur Marken setzen. Der Dienst haelt den
            // ganzen Verlauf vor; ohne diese Sperre kippte er beim ersten
            // Verbinden vollstaendig in die Nachrichten-App.
            for c in &chats {
                stand.insert(c.jid.clone(), c.last_time);
            }
            *eingerichtet = true;
            println!("Erstlauf: {} Chats als gelesen vermerkt", stand.len());
            return Vec::new();
        }

        let mut aus: Vec<NeueNachricht> = Vec::new();
        for c in chats {
            let marke = stand.get(&c.jid).copied().unwrap_or(0);
            if c.last_time <= marke {
                continue;
            }
            let Ok(verlauf) = dienst.nachrichten(&c.jid).await else {
                continue;
            };
            let titel = if c.name.is_empty() { c.jid.clone() } else { c.name.clone() };
            for m in verlauf.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
                if m.timestamp <= marke || m.from_me {
                    continue;
                }
                let kennung = if m.id.is_empty() {
                    format!("{}:{}", c.jid, m.timestamp)
                } else {
                    m.id.clone()
                };
                if !gesehen.insert(kennung) {
                    continue;
                }
                let text = if m.text.is_empty() {
                    if m.media_type.is_empty() {
                        continue;
                    }
                    format!("[{}]", m.media_type)
                } else {
                    m.text.clone()
                };
                // Gruppen erkennt man am Merkmal des Dienstes, nicht an
                // der Laenge der Kennung -- daran ist der WhatsApp-Port
                // schon einmal gescheitert.
                let absender = if c.is_group && !m.sender.is_empty() {
                    namen.get(&m.sender).cloned().unwrap_or_else(|| m.sender.clone())
                } else {
                    String::new()
                };
                aus.push(NeueNachricht {
                    chat: c.jid.clone(),
                    chat_name: titel.clone(),
                    absender,
                    text,
                    zeit: m.timestamp,
                });
            }
            stand.insert(c.jid.clone(), c.last_time);
        }
        // Die Liste der gesehenen Kennungen darf nicht ohne Ende wachsen.
        if gesehen.len() > 4000 {
            gesehen.clear();
        }
        aus.sort_by_key(|n| n.zeit);
        aus
    }
}
