//! Die Schleife einer Verbindung: sie haelt alles zusammen.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use zbus::object_server::SignalEmitter;

use crate::hintergrund::Hintergrund;
use crate::kanal::{Kanal, KanalZustand, Text};
use crate::verbindung::{Anfragen, Auftrag, GeteilterZustand, Verbindung};
use crate::{GRIFF_KONTAKT, IF_KANAL, STATUS_GETRENNT, STATUS_VERBINDET, STATUS_VERBUNDEN};

pub struct Lauf {
    pub bus: zbus::Connection,
    pub z: GeteilterZustand,
    pub auftraege: mpsc::UnboundedReceiver<Auftrag>,
    pub kanal_wuensche: mpsc::UnboundedReceiver<(u32, bool)>,
    pub senden_an: mpsc::UnboundedSender<Auftrag>,
    /// Kanalpfad -> Zustand, damit Nachrichten dorthin finden.
    pub kanalzustaende: HashMap<u32, Arc<Mutex<KanalZustand>>>,
}

impl Lauf {
    pub async fn laufen(mut self) {
        let protokoll = self.z.lock().await.protokoll.clone();
        let mut hintergrund: Option<Hintergrund> = None;
        let mut namen: HashMap<String, String> = HashMap::new();

        loop {
            // Auftraege haben Vorrang: sie kommen von der Oberflaeche und
            // sollen nicht hinter einer langen Abfrage warten.
            tokio::select! {
                biased;

                auftrag = self.auftraege.recv() => {
                    match auftrag {
                        Some(Auftrag::Verbinden) => {
                            if hintergrund.is_some() {
                                // Schon verbunden -- den Zustand noch
                                // einmal melden. Mission Control fragt
                                // mehrfach, und wer dann schweigt, gilt
                                // ihm als offline.
                                self.status_melden(STATUS_VERBUNDEN).await;
                                continue;
                            }
                            self.status_melden(STATUS_VERBINDET).await;
                            match Hintergrund::oeffnen(&protokoll).await {
                                Ok(h) => {
                                    // Erst der eigene Griff, dann alles
                                    // andere: ohne ihn gilt die
                                    // Verbindung dem Kontoverwalter als
                                    // nicht verbunden.
                                    let eigen = h.eigene_kennung().await;
                                    {
                                        let mut z = self.z.lock().await;
                                        let name = if eigen.is_empty() {
                                            format!("selbst-{protokoll}")
                                        } else {
                                            eigen.clone()
                                        };
                                        let g = z.kennungen.griff(&name);
                                        z.kennungen.name_setzen(g, &name);
                                        z.selbst = g;
                                        let p = z.protokoll.clone();
                                        z.kennungen.sichern(&p);
                                        eprintln!("{protokoll}: eigener Griff {g} ({name})");
                                    }
                                    namen = h.chats().await;
                                    eprintln!("{protokoll}: {} Chats bekannt", namen.len());
                                    self.namen_uebernehmen(&namen).await;
                                    hintergrund = Some(h);
                                    self.status_melden(STATUS_VERBUNDEN).await;
                                }
                                Err(e) => {
                                    eprintln!("{protokoll}: {e}");
                                    self.status_melden(STATUS_GETRENNT).await;
                                }
                            }
                        }
                        Some(Auftrag::Trennen) => {
                            // Der Draht bleibt, siehe Verbindung::disconnect.
                            self.status_melden(STATUS_GETRENNT).await;
                        }
                        Some(Auftrag::Senden { an, text }) => {
                            if let Some(h) = &hintergrund {
                                if let Err(e) = h.senden(&an, &text).await {
                                    eprintln!("{protokoll}: Senden: {e}");
                                }
                            }
                        }
                        None => return,
                    }
                }

                wunsch = self.kanal_wuensche.recv() => {
                    let Some((griff, melden)) = wunsch else { return };
                    self.kanal_sichern(griff).await;
                    if melden {
                        self.an_commhistory(griff, false).await;
                    }
                }

                nachrichten = async {
                    match hintergrund.as_mut() {
                        Some(h) => h.naechste(&namen).await,
                        None => {
                            // Ohne Hintergrund nicht heisslaufen.
                            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                            Vec::new()
                        }
                    }
                } => {
                    for n in nachrichten {
                        self.zustellen(n, &mut namen).await;
                    }
                }
            }
        }
    }

    async fn status_melden(&self, status: u32) {
        {
            let mut z = self.z.lock().await;
            if z.status == status {
                // Trotzdem melden: siehe oben.
            }
            z.status = status;
        }
        let pfad = self.z.lock().await.pfad.clone();
        if let Ok(emitter) = SignalEmitter::new(&self.bus, pfad) {
            let _ = Verbindung::tp_status_changed(&emitter, status, 0).await;
        }
        eprintln!("Zustand: {status}");
    }

    async fn namen_uebernehmen(&self, namen: &HashMap<String, String>) {
        let (pfad, geaendert) = {
            let mut z = self.z.lock().await;
            let mut geaendert: Vec<(u32, String)> = Vec::new();
            for (kennung, name) in namen {
                let g = z.kennungen.griff(kennung);
                if z.kennungen.name(g) != *name {
                    geaendert.push((g, name.clone()));
                }
                z.kennungen.name_setzen(g, name);
                z.chats.insert(kennung.clone(), name.clone());
            }
            let p = z.protokoll.clone();
            z.kennungen.sichern(&p);
            (z.pfad.clone(), geaendert)
        };

        // Seit InspectHandles die Kennung liefert, ist dies der Weg, auf
        // dem die Nachrichten-App den Namen erfaehrt.
        if geaendert.is_empty() {
            return;
        }
        if let Ok(pfad) = zbus::zvariant::ObjectPath::try_from(pfad) {
            if let Ok(emitter) = SignalEmitter::new(&self.bus, pfad) {
                let _ = crate::verbindung::Namen::aliases_changed(&emitter, geaendert).await;
            }
        }
    }

    /// Legt einen Kanal an, falls er fehlt, und meldet ihn.
    async fn kanal_sichern(&mut self, griff: u32) -> String {
        if let Some(pfad) = self.z.lock().await.kanaele.get(&griff) {
            return pfad.clone();
        }
        let (pfad, kennung) = {
            let mut z = self.z.lock().await;
            let pfad = match z.vorgemerkt.remove(&griff) {
                // Jemand hat den Pfad schon bekommen; ein zweiter waere
                // eine Adresse, unter der nie etwas erscheint.
                Some(p) => p,
                None => {
                    z.naechster_kanal += 1;
                    format!("{}/TextChannel{}", z.pfad, z.naechster_kanal)
                }
            };
            z.kanaele.insert(griff, pfad.clone());
            // Die KENNUNG, nicht der Anzeigename: Text.Send nimmt genau
            // dieses Feld als Empfaengeradresse, und ein Name ist keine.
            let kennung = z
                .kennungen
                .kennung(griff)
                .map(|s| s.to_string())
                .unwrap_or_else(|| z.kennungen.name(griff));
            (pfad, kennung)
        };

        let kz = Arc::new(Mutex::new(KanalZustand {
            pfad: pfad.clone(),
            griff,
            griffart: GRIFF_KONTAKT,
            kennung,
            ausstehend: Vec::new(),
            naechste_nummer: 0,
            geschlossen: false,
            zuletzt_gemeldet: std::time::Instant::now()
                - std::time::Duration::from_secs(3600),
        }));
        self.kanalzustaende.insert(griff, kz.clone());

        let server = self.bus.object_server();
        let _ = server.at(pfad.clone(), Kanal { z: kz.clone() }).await;
        let _ = server
            .at(
                pfad.clone(),
                Text { z: kz.clone(), senden: self.senden_kanal() },
            )
            .await;

        // NewChannel und NewChannels: Mission Control hoert auf beides,
        // je nach Alter der Schnittstelle.
        let (eigenschaften, verbindungspfad) = {
            let z = self.z.lock().await;
            (crate::kanal_eigenschaften(&z, griff, &pfad), z.pfad.clone())
        };
        if let Ok(emitter) = SignalEmitter::new(&self.bus, verbindungspfad) {
            if let Ok(p) = zbus::zvariant::ObjectPath::try_from(pfad.clone()) {
                let _ = Verbindung::new_channel(
                    &emitter,
                    p.clone(),
                    crate::IF_TEXT,
                    GRIFF_KONTAKT,
                    griff,
                    false,
                )
                .await;
                let _ = Anfragen::new_channels(&emitter, vec![(p, eigenschaften)]).await;
            }
        }
        eprintln!("Kanal angelegt: {pfad}");
        pfad
    }

    /// Ein Sender, der Sendewuensche aus einem Kanal in die Schleife traegt.
    fn senden_kanal(&self) -> mpsc::UnboundedSender<(String, String)> {
        let an = self.senden_an.clone();
        let (s, mut r) = mpsc::unbounded_channel::<(String, String)>();
        tokio::spawn(async move {
            while let Some((ziel, text)) = r.recv().await {
                let _ = an.send(Auftrag::Senden { an: ziel, text });
            }
        });
        s
    }

    async fn zustellen(&mut self, n: crate::hintergrund::NeueNachricht, namen: &mut HashMap<String, String>) {
        if !n.chat_name.is_empty() {
            namen.insert(n.chat.clone(), n.chat_name.clone());
        }
        let griff = {
            let mut z = self.z.lock().await;
            let g = z.kennungen.griff(&n.chat);
            if !n.chat_name.is_empty() {
                z.kennungen.name_setzen(g, &n.chat_name);
            }
            let p = z.protokoll.clone();
            z.kennungen.sichern(&p);
            g
        };
        let pfad = self.kanal_sichern(griff).await;

        // In einer Gruppe den Absender voranstellen -- die
        // Nachrichten-App zeigt sonst nur den Gruppennamen, und man weiss
        // nicht, wer spricht.
        let text = if n.absender.is_empty() {
            n.text
        } else {
            format!("<{}> {}", n.absender, n.text)
        };

        let (nummer, offen) = {
            let Some(kz) = self.kanalzustaende.get(&griff) else { return };
            let mut k = kz.lock().await;
            k.naechste_nummer += 1;
            let nummer = k.naechste_nummer;
            let offen = k.ausstehend.len();
            k.ausstehend.push((
                nummer,
                n.zeit as u32,
                griff,
                0,
                0,
                text.clone(),
            ));
            (nummer, offen)
        };

        if let Ok(emitter) = SignalEmitter::new(&self.bus, pfad.clone()) {
            let _ =
                Text::received(&emitter, nummer, n.zeit as u32, griff, 0, 0, &text).await;
        }

        // An CommHistory melden, wenn es noetig ist.
        //
        // Noetig ist es, wenn schon etwas Unbestaetigtes im Kanal lag --
        // dann hoert offenbar niemand mehr zu. Die Nachrichten-App
        // schliesst einen Kanal, sobald man die Unterhaltung verlaesst,
        // und ab da stapeln sich die Nachrichten unsichtbar. Genau das
        // war bei pybridge zu sehen.
        let lange_her = {
            let Some(kz) = self.kanalzustaende.get(&griff) else { return };
            let k = kz.lock().await;
            k.zuletzt_gemeldet.elapsed() > std::time::Duration::from_secs(60)
        };
        if offen > 0 || lange_her {
            if let Some(kz) = self.kanalzustaende.get(&griff) {
                kz.lock().await.zuletzt_gemeldet = std::time::Instant::now();
            }
            self.an_commhistory(griff, true).await;
        }
    }

    /// Meldet einen Kanal an CommHistory und, wenn gewuenscht, an die
    /// Nachrichten-App.
    ///
    /// pybridge startet dafuer einen eigenen Python-Prozess. Hier sind es
    /// zwei D-Bus-Aufrufe im selben Prozess – das war der Grund, warum
    /// beim Nachholen vieler Nachrichten die Last auf ueber sieben stieg.
    async fn an_commhistory(&self, griff: u32, nur_beobachten: bool) {
        let (pfad, eigenschaften, verbindungspfad) = {
            let z = self.z.lock().await;
            let Some(pfad) = z.kanaele.get(&griff).cloned() else { return };
            let e = crate::kanal_eigenschaften(&z, griff, &pfad);
            (pfad, e, z.pfad.clone())
        };
        let Ok(kanalpfad) = zbus::zvariant::ObjectPath::try_from(pfad) else { return };
        let Ok(connpfad) = zbus::zvariant::ObjectPath::try_from(verbindungspfad) else {
            return;
        };
        let konto = self.kontopfad().await;
        let Ok(kontopfad) = zbus::zvariant::ObjectPath::try_from(konto) else { return };

        let bus = self.bus.clone();
        let kanalpfad = kanalpfad.to_owned();
        let connpfad = connpfad.to_owned();
        let kontopfad = kontopfad.to_owned();

        // Nebenlaeufig und mit Frist.
        //
        // CommHistory antwortet auf diesem Geraet nicht einmal auf
        // Introspect, und zbus wartet ohne Zeitgrenze -- ein Aufruf in
        // der Schleife legte sie damit still. pybridge startet dafuer
        // einen Wegwerfprozess; hier genuegt eine eigene Aufgabe.
        //
        // Die Antwort interessiert ohnehin nicht: entweder CommHistory
        // nimmt den Kanal an oder nicht, ruecknehmen liesse sich nichts.
        tokio::spawn(async move {
            // Die Pfade muessen die Aufrufe ueberleben -- als
            // Zwischenwerte im Argument waeren sie vorher fort.
            let kp = kanalpfad.as_ref();
            let cp = connpfad.as_ref();
            let kop = kontopfad.as_ref();
            let kanaele = vec![(kp, eigenschaften)];
            let leer: HashMap<String, zbus::zvariant::OwnedValue> = HashMap::new();
            let keine: Vec<zbus::zvariant::ObjectPath> = Vec::new();
            let wurzel = zbus::zvariant::ObjectPath::try_from("/").unwrap();

            let args_beobachten = (&kop, &cp, &kanaele, &wurzel, &keine, &leer);
            let beobachten = bus.call_method(
                Some("org.freedesktop.Telepathy.Client.CommHistory"),
                "/org/freedesktop/Telepathy/Client/CommHistory",
                Some("org.freedesktop.Telepathy.Client.Observer"),
                "ObserveChannels",
                &args_beobachten,
            );
            match tokio::time::timeout(std::time::Duration::from_secs(20), beobachten)
                .await
            {
                Ok(Ok(_)) => eprintln!("CommHistory hat Griff {griff} angenommen"),
                Ok(Err(e)) => eprintln!("CommHistory: {e}"),
                Err(_) => eprintln!("CommHistory antwortet nicht (Griff {griff})"),
            }

            if nur_beobachten {
                return;
            }

            let args_zeigen = (&kop, &cp, &kanaele, &keine, 0u64, &leer);
            let zeigen = bus.call_method(
                Some("org.freedesktop.Telepathy.Client.Messaging"),
                "/org/freedesktop/Telepathy/Client/Messaging",
                Some("org.freedesktop.Telepathy.Client.Handler"),
                "HandleChannels",
                &args_zeigen,
            );
            match tokio::time::timeout(std::time::Duration::from_secs(20), zeigen).await {
                Ok(Ok(_)) => eprintln!("Nachrichten-App hat Griff {griff} angenommen"),
                Ok(Err(e)) => eprintln!("Nachrichten-App: {e}"),
                Err(_) => eprintln!("Nachrichten-App antwortet nicht (Griff {griff})"),
            }
        });
    }

    /// Der Pfad unseres Kontos beim Kontoverwalter.
    ///
    /// Er muss stimmen, sonst schreibt CommHistory den Verlauf unter ein
    /// fremdes Konto. Deshalb gefragt statt geraten.
    async fn kontopfad(&self) -> String {
        let protokoll = self.z.lock().await.protokoll.clone();
        let suche = format!("/{}/{protokoll}/", crate::CM_NAME);
        if let Ok(antwort) = self
            .bus
            .call_method(
                Some("org.freedesktop.Telepathy.AccountManager"),
                "/org/freedesktop/Telepathy/AccountManager",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(
                    "org.freedesktop.Telepathy.AccountManager",
                    "ValidAccounts",
                ),
            )
            .await
        {
            if let Ok(v) = antwort.body().deserialize::<zbus::zvariant::Value>() {
                if let zbus::zvariant::Value::Array(a) = v {
                    for e in a.iter() {
                        if let zbus::zvariant::Value::ObjectPath(p) = e {
                            let s = p.as_str().to_string();
                            if s.contains(&suche) {
                                return s;
                            }
                        }
                    }
                }
            }
        }
        format!(
            "/org/freedesktop/Telepathy/Account/{}/{protokoll}/{protokoll}0",
            crate::CM_NAME
        )
    }
}

pub const _K: u32 = IF_KANAL.len() as u32;
