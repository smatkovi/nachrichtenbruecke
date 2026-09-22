//! Der zweite Weg: die vorhandenen Python-Daemons von Telegram und Matrix.
//!
//! Sie sprechen Zeilen-JSON ueber einen Unix-Socket:
//!
//!     hin:    {"cmd": "get_dialogs", "args": {"limit": 100}}\n
//!     zurueck:{"dialogs": [...]}\n                 (Antwort)
//!             {"event": "new_message", "data": {}}\n (Ereignis)
//!
//! Eine Zeile mit "event" ist ein Ereignis, jede andere die Antwort auf
//! die zuletzt gestellte Frage -- Vorgangsnummern gibt es nicht. Antworten
//! muessen deshalb in der Reihenfolge der Fragen kommen, und genau eine
//! Frage darf offen sein. Darum der Mutex um das Fragen.
//!
//! Diese Daemons bleiben, wie sie sind. Sie zu ersetzen waere ein
//! zweites Projekt, und sie tun ihre Arbeit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Wo ein Daemon liegt und wie er gestartet wird.
pub struct Beschreibung {
    pub skript: &'static str,
    pub socket: &'static str,
}

pub fn beschreibung(protokoll: &str) -> Option<Beschreibung> {
    match protokoll {
        "telegram" => Some(Beschreibung {
            skript: "/opt/pytelegram/telegram_daemon.py",
            socket: ".pytelegram/daemon.sock",
        }),
        "matrix" => Some(Beschreibung {
            skript: "/opt/pymatrix/matrix_daemon.py",
            socket: ".pymatrix/daemon.sock",
        }),
        _ => None,
    }
}

fn socketpfad(rel: &str) -> PathBuf {
    let heim = std::env::var("HOME").unwrap_or_else(|_| "/home/user".into());
    PathBuf::from(heim).join(rel)
}

/// Eine offene Verbindung zu einem Daemon.
pub struct SocketDienst {
    protokoll: String,
    /// Fragen duerfen sich nicht ueberholen.
    fragen: Arc<Mutex<mpsc::UnboundedSender<(Value, oneshot::Sender<Value>)>>>,
}

impl SocketDienst {
    /// Startet den Daemon, wenn noetig, und verbindet sich.
    ///
    /// Der Daemon wird nur gestartet, wenn der Socket nicht antwortet --
    /// ein zweiter neben einem laufenden brachte bei WhatsApp schon
    /// einmal den Nachrichtenspeicher durcheinander.
    pub async fn verbinden(
        protokoll: &str,
        ereignisse: mpsc::UnboundedSender<Value>,
    ) -> Result<Self, String> {
        let b = beschreibung(protokoll).ok_or("unbekanntes Protokoll")?;
        let pfad = socketpfad(b.socket);

        if UnixStream::connect(&pfad).await.is_err() {
            if !std::path::Path::new(b.skript).exists() {
                return Err(format!("Daemon fehlt: {}", b.skript));
            }
            let _ = std::fs::remove_file(&pfad);
            eprintln!("{protokoll}: starte Daemon");
            let mut befehl = tokio::process::Command::new("/opt/wunderw/bin/python3.11");
            befehl.arg(b.skript);
            befehl.env("PYTHONIOENCODING", "utf-8");
            befehl.stdout(std::process::Stdio::null());
            befehl.stderr(std::process::Stdio::null());
            befehl.spawn().map_err(|e| e.to_string())?;
            // Auf den Socket warten -- der Daemon braucht auf diesem
            // Geraet seine Zeit.
            for _ in 0..60 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if pfad.exists() {
                    break;
                }
            }
        }

        let strom = UnixStream::connect(&pfad)
            .await
            .map_err(|e| format!("{}: {e}", pfad.display()))?;
        let (lesen, mut schreiben) = strom.into_split();

        let (sender, mut empfaenger) =
            mpsc::unbounded_channel::<(Value, oneshot::Sender<Value>)>();
        // Wer auf eine Antwort wartet, in der Reihenfolge der Fragen.
        let wartende: Arc<Mutex<Vec<oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(Vec::new()));

        // Schreiben
        let w = wartende.clone();
        tokio::spawn(async move {
            while let Some((frage, antwort_an)) = empfaenger.recv().await {
                w.lock().await.push(antwort_an);
                let zeile = format!("{frage}\n");
                if schreiben.write_all(zeile.as_bytes()).await.is_err() {
                    break;
                }
            }
        });

        // Lesen
        let w = wartende.clone();
        let name = protokoll.to_string();
        tokio::spawn(async move {
            let mut leser = BufReader::new(lesen).lines();
            loop {
                match leser.next_line().await {
                    Ok(Some(zeile)) => {
                        let Ok(v) = serde_json::from_str::<Value>(&zeile) else {
                            continue;
                        };
                        if v.get("event").is_some() {
                            let _ = ereignisse.send(v);
                        } else if let Some(an) = w.lock().await.pop() {
                            let _ = an.send(v);
                        }
                    }
                    _ => break,
                }
            }
            eprintln!("{name}: Daemon-Verbindung beendet");
            let _ = ereignisse.send(json!({"event": "disconnected"}));
        });

        Ok(SocketDienst {
            protokoll: protokoll.to_string(),
            fragen: Arc::new(Mutex::new(sender)),
        })
    }

    pub async fn fragen(&self, befehl: &str, args: Value) -> Result<Value, String> {
        let (an, von) = oneshot::channel();
        let mut frage = json!({"cmd": befehl});
        if !args.is_null() {
            frage["args"] = args;
        }
        {
            let s = self.fragen.lock().await;
            s.send((frage, an)).map_err(|_| "Daemon fort".to_string())?;
        }
        match tokio::time::timeout(Duration::from_secs(30), von).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err("keine Antwort".into()),
            Err(_) => Err("Zeitgrenze".into()),
        }
    }

    pub fn protokoll(&self) -> &str {
        &self.protokoll
    }
}

/// Die Antwort auf get_dialogs, so verschieden sie ausfaellt.
///
/// Telegram nennt sie "dialogs" mit "id"/"title", Matrix "rooms" mit
/// "room_id"/"name". Beides wird hier auf dasselbe gebracht, damit der
/// Rest der Bruecke nur eine Form kennt.
pub fn dialoge_lesen(protokoll: &str, v: &Value) -> Vec<(String, String)> {
    let mut aus = Vec::new();
    let feld = if protokoll == "matrix" { "rooms" } else { "dialogs" };
    let Some(liste) = v.get(feld).and_then(|x| x.as_array()) else {
        return aus;
    };
    for d in liste {
        let kennung = d
            .get("id")
            .or_else(|| d.get("room_id"))
            .map(wert_als_text)
            .unwrap_or_default();
        if kennung.is_empty() {
            continue;
        }
        let titel = d
            .get("title")
            .or_else(|| d.get("name"))
            .or_else(|| d.get("first_name"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        aus.push((kennung, titel));
    }
    aus
}

/// Kennungen kommen mal als Zahl, mal als Zeichenkette.
pub fn wert_als_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Aus einem Ereignis das herausholen, was eine Nachricht ausmacht.
pub fn nachricht_aus_ereignis(protokoll: &str, v: &Value) -> Option<HashMap<String, String>> {
    let art = v.get("event")?.as_str()?;
    let daten = v.get("data")?;
    let mut m = HashMap::new();

    if protokoll == "matrix" {
        if art != "room_event" {
            return None;
        }
        if daten.get("type")?.as_str()? != "m.room.message" {
            return None;
        }
        m.insert("chat".into(), wert_als_text(daten.get("room_id")?));
        m.insert("sender".into(), wert_als_text(daten.get("sender")?));
        let inhalt = daten.get("content")?;
        m.insert(
            "text".into(),
            inhalt.get("body").and_then(|x| x.as_str()).unwrap_or("").into(),
        );
        m.insert("zeit".into(), wert_als_text(daten.get("origin_server_ts")?));
        return Some(m);
    }

    if art != "new_message" && art != "message_edited" {
        return None;
    }
    if daten.get("out").and_then(|x| x.as_bool()).unwrap_or(false) {
        return None;
    }
    m.insert("chat".into(), wert_als_text(daten.get("chat_id")?));
    m.insert(
        "sender".into(),
        daten.get("sender_id").map(wert_als_text).unwrap_or_default(),
    );
    m.insert(
        "sender_name".into(),
        daten.get("sender_name").and_then(|x| x.as_str()).unwrap_or("").into(),
    );
    m.insert(
        "chat_name".into(),
        daten.get("chat_name").and_then(|x| x.as_str()).unwrap_or("").into(),
    );
    let text = daten.get("text").and_then(|x| x.as_str()).unwrap_or("");
    if text.is_empty() {
        let art = daten.get("media_type").and_then(|x| x.as_str()).unwrap_or("");
        if art.is_empty() {
            return None;
        }
        m.insert("text".into(), format!("[{art}]"));
    } else {
        m.insert("text".into(), text.to_string());
    }
    m.insert(
        "zeit".into(),
        daten.get("date").map(wert_als_text).unwrap_or_default(),
    );
    Some(m)
}
