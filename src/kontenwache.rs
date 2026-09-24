//! Haelt die Konten oben, die oben sein sollen.
//!
//! Faellt das Netz weg und kommt zurueck, bleibt ein Konto gelegentlich
//! tot liegen: die Telepathy-Konten haben keine Netzueberwachung, und
//! grammers, der Kern des Telegram-Dienstes, verbindet von sich aus nie
//! neu. In der Nachrichten-App sieht man davon nichts, man wartet nur
//! vergeblich auf Nachrichten.
//!
//! Die Wache sieht deshalb nach, wenn sich am Netz etwas tut, und
//! ausserdem alle paar Minuten. Sie fasst aber ausschliesslich Konten an,
//! bei denen Wunsch und Wirklichkeit auseinanderfallen:
//!
//! | RequestedPresence | CurrentPresence | was geschieht |
//! |---|---|---|
//! | available | available | nichts, es laeuft |
//! | available | offline   | anstossen, das ist der Ausfall |
//! | offline   | egal      | **nichts**, das ist eine Entscheidung |
//!
//! Die letzte Zeile ist der Grund, warum es diese Datei gibt. Wer seine
//! Konten abschaltet, will sie abgeschaltet haben; von aussen sieht das
//! genauso aus wie ein Ausfall, und wer beides verwechselt, schaltet
//! einem Menschen ungefragt die Erreichbarkeit wieder ein.

use std::time::Duration;

use zbus::zvariant::OwnedValue;

const AM: &str = "org.freedesktop.Telepathy.AccountManager";
const AM_PFAD: &str = "/org/freedesktop/Telepathy/AccountManager";
const IF_KONTO: &str = "org.freedesktop.Telepathy.Account";

/// Telepathy-Praesenz: 1 ist offline, 2 verfuegbar.
const OFFLINE: u32 = 1;
const VERFUEGBAR: u32 = 2;

/// Wie lange nach einem Netzereignis gewartet wird, bevor nachgesehen
/// wird. Ein frisch aufgebautes WLAN traegt noch nicht sofort.
const ANLAUF: Duration = Duration::from_secs(5);

/// Der langsame Durchlauf fuer den Fall, dass ein Dienst stirbt, ohne dass
/// sich am Netz etwas aendert.
const TAKT: Duration = Duration::from_secs(300);

pub fn starten(sitzung: zbus::Connection) {
    let fuers_netz = sitzung.clone();
    tokio::spawn(async move {
        if let Err(e) = am_netz_haengen(fuers_netz).await {
            eprintln!("Kontenwache: kein Netzsignal ({e})");
        }
    });
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TAKT).await;
            nachsehen(&sitzung, "Durchlauf").await;
        }
    });
}

/// Auf icd2 hoeren, den Verbindungsdienst von Harmattan.
///
/// Was in dem Signal steht, ist gleichgueltig: jede Meldung heisst "am
/// Netz hat sich etwas getan", und Nachsehen ist billig und folgenlos,
/// solange alles steht. Den Inhalt zu deuten hiesse, die Signatur von
/// icd2 zu raten.
async fn am_netz_haengen(sitzung: zbus::Connection) -> zbus::Result<()> {
    let system = zbus::Connection::system().await?;
    let regel = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("com.nokia.icd2")?
        .member("state_sig")?
        .build();
    let mut strom = zbus::MessageStream::for_match_rule(regel, &system, None).await?;
    eprintln!("Kontenwache: haengt an icd2");

    use futures_util::StreamExt;
    let mut zuletzt = std::time::Instant::now() - TAKT;
    while let Some(_meldung) = strom.next().await {
        // Ein Netzwechsel schickt mehrere Signale hintereinander.
        if zuletzt.elapsed() < Duration::from_secs(10) {
            continue;
        }
        zuletzt = std::time::Instant::now();
        tokio::time::sleep(ANLAUF).await;
        nachsehen(&sitzung, "Netzwechsel").await;
    }
    Ok(())
}

async fn eigenschaft(
    sitzung: &zbus::Connection,
    pfad: &str,
    name: &str,
) -> zbus::Result<OwnedValue> {
    let p = zbus::Proxy::new(sitzung, AM, pfad, "org.freedesktop.DBus.Properties").await?;
    p.call("Get", &(IF_KONTO, name)).await
}

/// Die eine Regel, auf die es ankommt.
///
/// Sie steht hier als eigene Funktion, damit ein Test sie festhaelt: der
/// Fall "jemand will offline sein" darf nie zu einem Eingriff fuehren,
/// auch nicht versehentlich, auch nicht spaeter.
pub fn eingreifen(gewuenscht: u32, tatsaechlich: u32) -> bool {
    gewuenscht == VERFUEGBAR && tatsaechlich == OFFLINE
}

/// Die Praesenzart aus einer Eigenschaft (u, s, s) herausholen.
fn art(wert: &OwnedValue) -> Option<u32> {
    let s = zbus::zvariant::Structure::try_from(wert.clone()).ok()?;
    let felder = s.fields();
    u32::try_from(felder.first()?.try_clone().ok()?).ok()
}

async fn nachsehen(sitzung: &zbus::Connection, anlass: &str) {
    let verwalter = match zbus::Proxy::new(
        sitzung,
        AM,
        AM_PFAD,
        "org.freedesktop.DBus.Properties",
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Kontenwache: Kontoverwaltung nicht erreichbar ({e})");
            return;
        }
    };
    let konten: OwnedValue = match verwalter.call("Get", &(AM, "ValidAccounts")).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Kontenwache: Kontenliste nicht lesbar ({e})");
            return;
        }
    };
    let pfade: Vec<zbus::zvariant::OwnedObjectPath> = match konten.try_into() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Kontenwache: Kontenliste unverstaendlich ({e})");
            return;
        }
    };

    for p in pfade {
        let pfad = p.as_str();
        if !pfad.contains("/bruecke/") {
            continue;
        }
        let gewuenscht = eigenschaft(sitzung, pfad, "RequestedPresence").await.ok();
        let tatsaechlich = eigenschaft(sitzung, pfad, "CurrentPresence").await.ok();
        let (Some(g), Some(t)) = (gewuenscht.as_ref().and_then(art), tatsaechlich.as_ref().and_then(art))
        else {
            continue;
        };
        if !eingreifen(g, t) {
            continue;
        }
        let name = pfad.rsplit('/').nth(1).unwrap_or(pfad);
        eprintln!("Kontenwache: {name} soll verbunden sein, ist es aber nicht ({anlass}) -- stosse an");
        anstossen(sitzung, pfad).await;
    }
}

/// Anstossen heisst: einmal abmelden und wieder anmelden.
///
/// Die Eigenschaft steht schon auf "verfuegbar"; sie noch einmal auf
/// denselben Wert zu setzen aendert nichts und loest deshalb auch nichts
/// aus. Erst der Wechsel bewegt die Kontoverwaltung.
async fn anstossen(sitzung: &zbus::Connection, pfad: &str) {
    let setzen = |art: u32| async move {
        let p = match zbus::Proxy::new(sitzung, AM, pfad, "org.freedesktop.DBus.Properties").await {
            Ok(p) => p,
            Err(_) => return,
        };
        let wert = zbus::zvariant::Value::from((art, "".to_string(), "".to_string()));
        let _: Result<(), _> = p.call("Set", &(IF_KONTO, "RequestedPresence", wert)).await;
    };
    setzen(OFFLINE).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    setzen(VERFUEGBAR).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ein abgeschaltetes Konto bleibt abgeschaltet.
    ///
    /// Von aussen sieht "ausgeschaltet" genauso aus wie "ausgefallen".
    /// Genau diese Verwechslung hat einem Menschen schon einmal ungefragt
    /// die Erreichbarkeit wieder eingeschaltet.
    #[test]
    fn offline_ist_eine_entscheidung() {
        // gewuenscht offline: niemals anfassen, egal was gerade ist
        assert!(!eingreifen(OFFLINE, OFFLINE));
        assert!(!eingreifen(OFFLINE, VERFUEGBAR));
        // laeuft: nichts zu tun
        assert!(!eingreifen(VERFUEGBAR, VERFUEGBAR));
        // soll oben sein, ist unten: das ist der Ausfall
        assert!(eingreifen(VERFUEGBAR, OFFLINE));
        // unbekannte Werte fuehren zu nichts
        assert!(!eingreifen(0, 0));
        assert!(!eingreifen(3, OFFLINE));
    }
}
