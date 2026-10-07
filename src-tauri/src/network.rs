use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use iroh::{Endpoint, SecretKey, endpoint::presets};
use iroh_tickets::endpoint::EndpointTicket;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    commands::AppState,
    config::{AppSettings, PairInvite, PeerConfig, encode_pair_code},
    crdt::CrdtManager,
    vault::{self, Manifest, NoteMeta},
};

const ALPN: &[u8] = b"lownotes/sync/4";
const IMAGE_ALPN: &[u8] = b"lownotes/sync/3";
const LEGACY_ALPN: &[u8] = b"lownotes/sync/2";
const MAX_PACKET_BYTES: usize = 24 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairInfo {
    pub pair_code: String,
    pub endpoint_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum NetworkEventPayload {
    Ready {
        pair_code: String,
        endpoint_id: String,
    },
    Syncing {
        peer: String,
    },
    Synced {
        peer: String,
        changed: usize,
        direct: Option<bool>,
    },
    PairRequested {
        request_id: String,
        peer: PeerConfig,
    },
    PairApproved {
        peer: PeerConfig,
    },
    PairRejected {
        peer: String,
    },
    RemoteCrdtUpdate {
        note_path: String,
        update: Vec<u8>,
    },
    Conflict {
        note_path: String,
        conflict_path: String,
    },
    RemoteAwareness {
        note_path: String,
        update: Vec<u8>,
    },
    Error {
        peer: Option<String>,
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Packet {
    PairRequest {
        token: String,
        requester: PeerConfig,
    },
    PairDecision {
        accepted: bool,
        responder: Option<PeerConfig>,
    },
    Manifest(Manifest),
    Put {
        meta: NoteMeta,
        content: Vec<u8>,
    },
    ImagePut {
        meta: NoteMeta,
        content_base64: String,
    },
    JsonPut {
        meta: NoteMeta,
        content: serde_json::Value,
    },
    Request {
        path: String,
    },
    CrdtUpdate {
        note_path: String,
        update: Vec<u8>,
    },
    Awareness {
        note_path: String,
        update: Vec<u8>,
    },
    Delete {
        path: String,
    },
    Done,
}

enum NetworkCommand {
    SyncNow,
    RequestPair(PairInvite),
    AnswerPair { request_id: String, accept: bool },
    BroadcastCrdt { note_path: String, update: Vec<u8> },
    BroadcastAwareness { note_path: String, update: Vec<u8> },
    BroadcastDelete { note_path: String },
}

#[derive(Clone)]
pub struct NetworkIdentity {
    pub device_name: String,
    pub secret_key: SecretKey,
    pub pairing_token: String,
    pub vault_id: String,
    pub vault_name: String,
}

pub struct NetworkService {
    peers: Arc<RwLock<Vec<PeerConfig>>>,
    commands: tokio::sync::mpsc::UnboundedSender<NetworkCommand>,
    pair_info: Arc<RwLock<Option<PairInfo>>>,
}

impl NetworkService {
    pub fn start(
        vault: PathBuf,
        identity: NetworkIdentity,
        initial_peers: Vec<PeerConfig>,
        app: AppHandle,
        settings: Arc<RwLock<AppSettings>>,
    ) -> Self {
        let peers = Arc::new(RwLock::new(initial_peers));
        let worker_peers = peers.clone();
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let pair_info = Arc::new(RwLock::new(None::<PairInfo>));
        let worker_pair_info = pair_info.clone();
        let worker_settings = settings.clone();

        std::thread::Builder::new()
            .name("lownotes-network".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .thread_name("lownotes-io")
                    .build();

                match runtime {
                    Ok(rt) => {
                        if let Err(err) = rt.block_on(run_network(
                            vault,
                            identity,
                            worker_peers,
                            worker_pair_info,
                            command_rx,
                            app.clone(),
                            worker_settings,
                        )) {
                            eprintln!("[p2p] network unavailable: {err:#}");
                            let _ = app.emit(
                                "p2p:error",
                                NetworkEventPayload::Error {
                                    peer: None,
                                    message: "errors.networkUnavailable".to_string(),
                                },
                            );
                        }
                    }
                    Err(err) => {
                        eprintln!("[p2p] failed to create network runtime: {err}");
                        let _ = app.emit(
                            "p2p:error",
                            NetworkEventPayload::Error {
                                peer: None,
                                message: "errors.networkRuntime".to_string(),
                            },
                        );
                    }
                }
            })
            .expect("failed to spawn network thread");

        Self {
            peers,
            commands: command_tx,
            pair_info,
        }
    }

    pub fn pair_info(&self) -> Option<PairInfo> {
        self.pair_info.read().clone()
    }

    pub fn update_peers(&self, peers: Vec<PeerConfig>) {
        *self.peers.write() = peers;
    }

    pub fn sync_now(&self) {
        let _ = self.commands.send(NetworkCommand::SyncNow);
    }

    pub fn request_pair(&self, invite: PairInvite) {
        let _ = self.commands.send(NetworkCommand::RequestPair(invite));
    }

    pub fn answer_pair(&self, request_id: String, accept: bool) {
        let _ = self
            .commands
            .send(NetworkCommand::AnswerPair { request_id, accept });
    }

    pub fn broadcast_crdt_update(&self, note_path: String, update: Vec<u8>) {
        let _ = self
            .commands
            .send(NetworkCommand::BroadcastCrdt { note_path, update });
    }

    pub fn broadcast_awareness(&self, note_path: String, update: Vec<u8>) {
        let _ = self
            .commands
            .send(NetworkCommand::BroadcastAwareness { note_path, update });
    }

    pub fn broadcast_delete(&self, note_path: String) {
        let _ = self
            .commands
            .send(NetworkCommand::BroadcastDelete { note_path });
    }
}

async fn run_network(
    vault: PathBuf,
    identity: NetworkIdentity,
    peers: Arc<RwLock<Vec<PeerConfig>>>,
    pair_info: Arc<RwLock<Option<PairInfo>>>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<NetworkCommand>,
    app: AppHandle,
    settings: Arc<RwLock<AppSettings>>,
) -> anyhow::Result<()> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(identity.secret_key.clone())
        .alpns(vec![ALPN.to_vec(), IMAGE_ALPN.to_vec(), LEGACY_ALPN.to_vec()])
        .bind()
        .await?;

    publish_pair_code(&endpoint, &identity, &app, &pair_info);

    let online_ep = endpoint.clone();
    let online_id = identity.clone();
    let online_app = app.clone();
    let online_pair_info = pair_info.clone();
    tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(10), online_ep.online()).await;
        publish_pair_code(&online_ep, &online_id, &online_app, &online_pair_info);
    });

    let sync_in_flight = Arc::new(parking_lot::Mutex::new(HashSet::<String>::new()));

    // Periodic reconciliation: keep vaults converged even after restarts or
    // missed CRDT broadcasts while a peer was offline.
    let tick_ep = endpoint.clone();
    let tick_vault = vault.clone();
    let tick_peers = peers.clone();
    let tick_app = app.clone();
    let tick_in_flight = sync_in_flight.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await;
        loop {
            interval.tick().await;
            let known: Vec<PeerConfig> = tick_peers.read().clone();
            for peer in known {
                spawn_sync(
                    tick_ep.clone(),
                    tick_vault.clone(),
                    peer,
                    tick_app.clone(),
                    tick_in_flight.clone(),
                );
            }
        }
    });

    let pending_answers = Arc::new(tokio::sync::Mutex::new(HashMap::<
        String,
        tokio::sync::oneshot::Sender<bool>,
    >::new()));

    // Accept loop
    let accept_ep = endpoint.clone();
    let accept_peers = peers.clone();
    let accept_app = app.clone();
    let accept_vault = vault.clone();
    let accept_token = identity.pairing_token.clone();
    let accept_answers = pending_answers.clone();
    let accept_ident = identity.clone();
    let accept_settings = settings.clone();
    let accept_sync = sync_in_flight.clone();
    tokio::spawn(async move {
        while let Some(incoming) = accept_ep.accept().await {
            let p_peers = accept_peers.clone();
            let p_app = accept_app.clone();
            let p_vault = accept_vault.clone();
            let p_token = accept_token.clone();
            let p_answers = accept_answers.clone();
            let p_ident = accept_ident.clone();
            let p_settings = accept_settings.clone();
            let p_sync = accept_sync.clone();

            let p_ep = accept_ep.clone();
            tokio::spawn(async move {
                let res = handle_incoming_connection(
                    p_ep,
                    incoming,
                    p_peers,
                    p_app,
                    p_vault,
                    p_token,
                    p_answers,
                    p_ident,
                    p_settings,
                    p_sync,
                )
                .await;
                if let Err(e) = res {
                    eprintln!("[p2p accept error]: {e}");
                }
            });
        }
    });

    // Reconcile immediately after a restart; the periodic pass remains a
    // fallback for peers that were unavailable at launch.
    for peer in peers.read().clone() {
        spawn_sync(endpoint.clone(), vault.clone(), peer, app.clone(), sync_in_flight.clone());
    }

    // Command loop
    let pair_in_flight = Arc::new(parking_lot::Mutex::new(HashSet::<String>::new()));

    while let Some(cmd) = commands.recv().await {
        match cmd {
            NetworkCommand::SyncNow => {
                let known: Vec<PeerConfig> = peers.read().clone();
                for peer in known {
                    spawn_sync(
                        endpoint.clone(),
                        vault.clone(),
                        peer,
                        app.clone(),
                        sync_in_flight.clone(),
                    );
                }
            }
            NetworkCommand::RequestPair(invite) => {
                let target_id = invite.peer.endpoint_id.clone();
                let inserted = pair_in_flight.lock().insert(target_id.clone());
                if !inserted {
                    continue;
                }
                let ep = endpoint.clone();
                let my_peers = peers.clone();
                let my_app = app.clone();
                let my_ident = identity.clone();
                let my_vault = vault.clone();
                let my_settings = settings.clone();
                let my_sync = sync_in_flight.clone();
                let in_flight = pair_in_flight.clone();

                tokio::spawn(async move {
                    let res = dial_pair(ep, invite, my_ident, my_peers.clone(), my_app.clone(), my_vault, my_settings, my_sync).await;
                    in_flight.lock().remove(&target_id);
                    if let Err(e) = res {
                        eprintln!("[p2p] pair failed: {e:#}");
                        let _ = my_app.emit(
                            "p2p:error",
                            NetworkEventPayload::Error {
                                peer: Some(target_id),
                                message: "errors.pairFailed".to_string(),
                            },
                        );
                    }
                });
            }
            NetworkCommand::AnswerPair { request_id, accept } => {
                let mut map = pending_answers.lock().await;
                if let Some(sender) = map.remove(&request_id) {
                    let _ = sender.send(accept);
                }
            }
            NetworkCommand::BroadcastCrdt { note_path, update } => {
                let known: Vec<PeerConfig> = peers.read().clone();
                for peer in known {
                    let ep = endpoint.clone();
                    let n_path = note_path.clone();
                    let u_bytes = update.clone();
                    tokio::spawn(async move {
                        let _ = send_crdt_to_peer(ep, peer, n_path, u_bytes).await;
                    });
                }
            }
            NetworkCommand::BroadcastAwareness { note_path, update } => {
                let known: Vec<PeerConfig> = peers.read().clone();
                for peer in known {
                    let ep = endpoint.clone();
                    let n_path = note_path.clone();
                    let u_bytes = update.clone();
                    tokio::spawn(async move {
                        let _ = send_awareness_to_peer(ep, peer, n_path, u_bytes).await;
                    });
                }
            }
            NetworkCommand::BroadcastDelete { note_path } => {
                let known: Vec<PeerConfig> = peers.read().clone();
                for peer in known {
                    let ep = endpoint.clone();
                    let n_path = note_path.clone();
                    tokio::spawn(async move {
                        let _ = send_delete_to_peer(ep, peer, n_path).await;
                    });
                }
            }
        }
    }

    Ok(())
}
fn publish_pair_code(
    endpoint: &Endpoint,
    identity: &NetworkIdentity,
    app: &AppHandle,
    pair_info: &Arc<RwLock<Option<PairInfo>>>,
) {
    let addr = endpoint.addr();
    let ticket = EndpointTicket::new(addr);
    let endpoint_id = endpoint.id().to_string();
    let invite = PairInvite {
        peer: PeerConfig {
            name: identity.device_name.clone(),
            endpoint_id: endpoint_id.clone(),
            ticket: ticket.to_string(),
        },
        token: identity.pairing_token.clone(),
        vault_id: identity.vault_id.clone(),
        vault_name: identity.vault_name.clone(),
    };
    if let Ok(code) = encode_pair_code(&invite) {
        *pair_info.write() = Some(PairInfo {
            pair_code: code.clone(),
            endpoint_id: endpoint_id.clone(),
        });
        let _ = app.emit(
            "p2p:ready",
            NetworkEventPayload::Ready {
                pair_code: code,
                endpoint_id,
            },
        );
    }
}

async fn handle_incoming_connection(
    endpoint: Endpoint,
    incoming: iroh::endpoint::Incoming,
    peers: Arc<RwLock<Vec<PeerConfig>>>,
    app: AppHandle,
    vault: PathBuf,
    pairing_token: String,
    pending_answers: Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
    identity: NetworkIdentity,
    settings: Arc<RwLock<AppSettings>>,
    sync_in_flight: Arc<parking_lot::Mutex<HashSet<String>>>,
) -> anyhow::Result<()> {
    let connection = incoming.await?;
    let remote_id = connection.remote_id();
    let (mut send, mut recv) = connection.accept_bi().await?;

    let packet: Packet = recv_packet(&mut recv).await?;

    match packet {
        Packet::PairRequest { token, requester } => {
            let already_known = peers.read().iter().any(|p| p.endpoint_id == remote_id.to_string());
            let (accepted, request_id) = if already_known {
                (true, None)
            } else if token != pairing_token {
                (false, None)
            } else {
                let req_id = remote_id.to_string();
                let (tx, rx) = tokio::sync::oneshot::channel();
                {
                    let mut lock = pending_answers.lock().await;
                    if let Some(prev) = lock.insert(req_id.clone(), tx) {
                        let _ = prev.send(false);
                    }
                }
                let _ = app.emit(
                    "p2p:pair-requested",
                    NetworkEventPayload::PairRequested {
                        request_id: req_id.clone(),
                        peer: requester.clone(),
                    },
                );

                let answer = tokio::time::timeout(Duration::from_secs(120), rx)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or(false);

                {
                    let mut lock = pending_answers.lock().await;
                    lock.remove(&req_id);
                }
                (answer, Some(req_id))
            };

            let responder_info = if accepted {
                Some(PeerConfig {
                    name: identity.device_name,
                    endpoint_id: endpoint.id().to_string(),
                    ticket: EndpointTicket::new(endpoint.addr()).to_string(),
                })
            } else {
                None
            };

            send_packet(&mut send, &Packet::PairDecision { accepted, responder: responder_info }).await?;
            send.finish()?;
            let _ = tokio::time::timeout(Duration::from_secs(5), send.stopped()).await;

            if accepted && !already_known {
                peers.write().push(requester.clone());
                persist_peer(&settings, &identity.vault_id, &requester);
                let _ = app.emit(
                    "p2p:pair-approved",
                    NetworkEventPayload::PairApproved {
                        peer: requester.clone(),
                    },
                );
                spawn_sync(
                    endpoint.clone(),
                    vault.clone(),
                    requester.clone(),
                    app.clone(),
                    sync_in_flight.clone(),
                );
            } else if !accepted {
                if let Some(r_id) = request_id {
                    let _ = app.emit(
                        "p2p:pair-rejected",
                        NetworkEventPayload::PairRejected {
                            peer: r_id,
                        },
                    );
                }
            }
            connection.close(0u32.into(), b"pair complete");
        }
        Packet::Manifest(remote_manifest) => {
            let peer = peers
                .read()
                .iter()
                .find(|p| p.endpoint_id == remote_id.to_string())
                .cloned()
                .context("errors.unauthorizedDevice")?;

            let changed = serve_sync(&mut send, &mut recv, &vault, remote_manifest, Some(&app), connection.alpn() != LEGACY_ALPN, connection.alpn() == ALPN).await?;
            connection.close(0u32.into(), b"sync complete");

            let direct = connection_is_direct(&connection);
            let _ = app.emit(
                "p2p:synced",
                NetworkEventPayload::Synced {
                    peer: peer.name,
                    changed,
                    direct,
                },
            );
        }
        Packet::CrdtUpdate { note_path, update } => {
            let is_peer = peers.read().iter().any(|p| p.endpoint_id == remote_id.to_string());
            if is_peer {
                let state = app.state::<AppState>();
                let merged = state.crdt.apply_update(&vault, &note_path, &update)?;
                if merged.changed {
                    let _ = app.emit("p2p:crdt-update", NetworkEventPayload::RemoteCrdtUpdate {
                        note_path, update: merged.state,
                    });
                }
            }
        }
        Packet::Awareness { note_path, update } => {
            let is_peer = peers.read().iter().any(|p| p.endpoint_id == remote_id.to_string());
            if is_peer {
                let _ = app.emit(
                    "p2p:awareness",
                    NetworkEventPayload::RemoteAwareness { note_path, update },
                );
            }
        }
        Packet::Delete { path } => {
            let is_peer = peers.read().iter().any(|p| p.endpoint_id == remote_id.to_string());
            if is_peer {
                let _ = vault::delete_item(&vault, &path);
                let _ = app.state::<AppState>().crdt.remove_doc(&vault, &path);
                let _ = app.emit("p2p:synced", NetworkEventPayload::Synced {
                    peer: remote_id.to_string(), changed: 1, direct: None,
                });
            }
        }
        _ => bail!("errors.unexpectedPacketStart"),
    }

    Ok(())
}

async fn dial_pair(
    endpoint: Endpoint,
    invite: PairInvite,
    identity: NetworkIdentity,
    peers: Arc<RwLock<Vec<PeerConfig>>>,
    app: AppHandle,
    vault: PathBuf,
    settings: Arc<RwLock<AppSettings>>,
    sync_in_flight: Arc<parking_lot::Mutex<HashSet<String>>>,
) -> anyhow::Result<()> {
    let addr = invite.peer.endpoint_addr()?;
    let connection = endpoint.connect(addr, LEGACY_ALPN).await?;
    let (mut send, mut recv) = connection.open_bi().await?;

    let my_requester = PeerConfig {
        name: identity.device_name,
        endpoint_id: endpoint.id().to_string(),
        ticket: EndpointTicket::new(endpoint.addr()).to_string(),
    };

    send_packet(
        &mut send,
        &Packet::PairRequest {
            token: invite.token,
            requester: my_requester,
        },
    )
    .await?;

    let response: Packet = tokio::time::timeout(Duration::from_secs(120), recv_packet(&mut recv))
        .await
        .context("errors.pairTimeout")??;

    match response {
        Packet::PairDecision { accepted: true, responder } => {
            // Prefer the fresh ticket sent by the responder (includes relay
            // addresses discovered after the invite code was generated).
            let peer = responder.unwrap_or(invite.peer);
            peers.write().push(peer.clone());
            persist_peer(&settings, &identity.vault_id, &peer);
            let _ = app.emit(
                "p2p:pair-approved",
                NetworkEventPayload::PairApproved {
                    peer: peer.clone(),
                },
            );
            spawn_sync(
                endpoint.clone(),
                vault.clone(),
                peer.clone(),
                app.clone(),
                sync_in_flight.clone(),
            );
            connection.close(0u32.into(), b"paired");
            Ok(())
        }
        Packet::PairDecision { accepted: false, .. } => {
            bail!("errors.pairRejected");
        }
        _ => bail!("errors.invalidPairResponse"),
    }
}

fn persist_peer(settings: &Arc<RwLock<AppSettings>>, vault_id: &str, peer: &PeerConfig) {
    let mut s = settings.write();
    if let Some(vault) = s.vaults.iter_mut().find(|v| v.id == vault_id) {
        if !vault.peers.iter().any(|p| p.endpoint_id == peer.endpoint_id) {
            vault.peers.push(peer.clone());
            let _ = s.save();
        }
    }
}

fn spawn_sync(
    endpoint: Endpoint,
    vault: PathBuf,
    peer: PeerConfig,
    app: AppHandle,
    in_flight: Arc<parking_lot::Mutex<HashSet<String>>>,
) {
    let target = peer.endpoint_id.clone();
    if !in_flight.lock().insert(target.clone()) {
        return;
    }

    tokio::spawn(async move {
        let _ = app.emit(
            "p2p:syncing",
            NetworkEventPayload::Syncing {
                peer: peer.name.clone(),
            },
        );

        let res = dial_sync(endpoint.clone(), vault.clone(), peer.clone(), Some(&app)).await;
        in_flight.lock().remove(&target);

        match res {
            Ok((changed, direct, conflict_created)) => {
                let _ = app.emit(
                    "p2p:synced",
                    NetworkEventPayload::Synced {
                        peer: peer.name.clone(),
                        changed,
                        direct,
                    },
                );
                // The conflict copy was created after this pass's manifest.
                // Send it to the other computer without waiting for the tick.
                if conflict_created {
                    spawn_sync(endpoint, vault, peer, app, in_flight);
                }
            }
            Err(e) => {
                eprintln!("[p2p] sync failed: {e:#}");
                let _ = app.emit(
                    "p2p:error",
                    NetworkEventPayload::Error {
                        peer: Some(peer.name),
                        message: "errors.syncFailed".to_string(),
                    },
                );
            }
        }
    });
}

async fn dial_sync(
    endpoint: Endpoint,
    vault: PathBuf,
    peer: PeerConfig,
    app: Option<&AppHandle>,
) -> anyhow::Result<(usize, Option<bool>, bool)> {
    let addr = peer.endpoint_addr()?;
    let connection = match endpoint.connect(addr.clone(), ALPN).await {
        Ok(connection) => connection,
        Err(_) => match endpoint.connect(addr.clone(), IMAGE_ALPN).await {
            Ok(connection) => connection,
            Err(_) => endpoint.connect(addr, LEGACY_ALPN).await?,
        },
    };
    let (mut send, mut recv) = connection.open_bi().await?;

    let images_supported = connection.alpn() != LEGACY_ALPN;
    let metadata_supported = connection.alpn() == ALPN;
    let my_manifest = sync_manifest(&vault, images_supported, metadata_supported)?;
    send_packet(&mut send, &Packet::Manifest(my_manifest.clone())).await?;

    let mut changed = 0;
    let mut conflict_created = false;
    loop {
        let packet: Packet = recv_packet(&mut recv).await?;
        match packet {
            Packet::Request { path } => {
                let meta = my_manifest.get(&path).context("errors.metaMissing")?;
                send_sync_content(&mut send, &vault, meta, metadata_supported).await?;
            }
            packet @ (Packet::Put { .. } | Packet::ImagePut { .. } | Packet::JsonPut { .. }) => {
                let (meta, content) = unpack_sync_content(packet, images_supported, metadata_supported)?;
                let outcome = write_sync_content(&vault, &meta.path, &content, app)?;
                if outcome.changed {
                    changed += 1;
                }
                conflict_created |= outcome.conflict_created;
            }
            Packet::Done => break,
            _ => bail!("errors.unexpectedPacketSync"),
        }
    }

    let direct = connection_is_direct(&connection);
    connection.close(0u32.into(), b"sync done");
    Ok((changed, direct, conflict_created))
}

async fn serve_sync(
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
    vault: &Path,
    remote_manifest: Manifest,
    app: Option<&AppHandle>,
    images_supported: bool,
    metadata_supported: bool,
) -> anyhow::Result<usize> {
    let local_manifest = sync_manifest(vault, images_supported, metadata_supported)?;
    if !images_supported && remote_manifest.keys().any(|path| crate::local_images::is_sync_image(path)) {
        bail!("errors.unexpectedPacketSync");
    }
    if !metadata_supported && remote_manifest.contains_key(crate::link_operations::RELATIVE_PATH) {
        bail!("errors.unexpectedPacketSync");
    }
    let mut changed = 0;

    // Send notes that remote doesn't have or remote has older
    for (path, local_meta) in &local_manifest {
        if skip_markdown_sync(path, &local_manifest, &remote_manifest) { continue; }
        let needs_send = match remote_manifest.get(path) {
            None => true,
            Some(remote_meta) if is_crdt_state(path) || crate::links::is_sync_metadata(path) => local_meta.hash != remote_meta.hash,
            Some(remote_meta) => local_meta.modified_ms > remote_meta.modified_ms && local_meta.hash != remote_meta.hash,
        };

        if needs_send {
            send_sync_content(send, vault, local_meta, metadata_supported).await?;
        }
    }

    // Request notes that remote has newer
    for (path, remote_meta) in &remote_manifest {
        if skip_markdown_sync(path, &local_manifest, &remote_manifest) { continue; }
        let needs_request = match local_manifest.get(path) {
            None => true,
            Some(local_meta) if is_crdt_state(path) || crate::links::is_sync_metadata(path) => remote_meta.hash != local_meta.hash,
            Some(local_meta) => remote_meta.modified_ms > local_meta.modified_ms && remote_meta.hash != local_meta.hash,
        };

        if needs_request {
            send_packet(send, &Packet::Request { path: path.clone() }).await?;
            let packet: Packet = recv_packet(recv).await?;
            let (meta, content) = unpack_sync_content(packet, images_supported, metadata_supported)?;
            if meta.path != *path { bail!("errors.unexpectedPacketSync"); }
            if write_sync_content(vault, &meta.path, &content, app)?.changed {
                changed += 1;
            }
        }
    }

    send_packet(send, &Packet::Done).await?;
    // Flush gracefully: CONNECTION_CLOSE right after a write can drop the
    // buffered Done packet, aborting the dialer with "connection lost".
    send.finish()?;
    let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
    Ok(changed)
}

fn sync_manifest(root: &Path, images_supported: bool, metadata_supported: bool) -> anyhow::Result<Manifest> {
    let mut manifest = vault::build_manifest(root)?;
    if !images_supported { manifest.retain(|path, _| !crate::local_images::is_sync_image(path)); }
    if !metadata_supported { manifest.remove(crate::link_operations::RELATIVE_PATH); }
    Ok(manifest)
}

async fn send_sync_content(send: &mut iroh::endpoint::SendStream, root: &Path, meta: &NoteMeta, metadata_supported: bool) -> anyhow::Result<()> {
    let content = read_sync_content(root, &meta.path)?;
    let packet = if metadata_supported && crate::links::is_sync_metadata(&meta.path) {
        Packet::JsonPut { meta: meta.clone(), content: serde_json::from_slice(&content)? }
    } else if crate::local_images::is_sync_image(&meta.path) {
        Packet::ImagePut { meta: meta.clone(), content_base64: STANDARD.encode(content) }
    } else { Packet::Put { meta: meta.clone(), content } };
    send_packet(send, &packet).await
}

fn unpack_sync_content(packet: Packet, images_supported: bool, metadata_supported: bool) -> anyhow::Result<(NoteMeta, Vec<u8>)> {
    match packet {
        Packet::JsonPut { meta, content } if metadata_supported && crate::links::is_sync_metadata(&meta.path) => {
            let bytes = serde_json::to_vec_pretty(&content)?;
            if bytes.len() > crate::link_operations::MAX_BYTES { bail!("invalid link metadata size"); }
            Ok((meta, bytes))
        }
        Packet::Put { meta, content } if !crate::local_images::is_sync_image(&meta.path)
            && (metadata_supported || meta.path != crate::link_operations::RELATIVE_PATH) => Ok((meta, content)),
        Packet::ImagePut { meta, content_base64 } if images_supported => {
            let name = crate::local_images::sync_name(&meta.path)?;
            let (id, _) = crate::local_images::image_name(name)?;
            if meta.hash != id || meta.size > crate::local_images::MAX_STORED_BYTES as u64
                || content_base64.len() > crate::local_images::MAX_STORED_BYTES.div_ceil(3) * 4 {
                bail!("errors.localImageInvalid");
            }
            let content = STANDARD.decode(content_base64)?;
            if content.len() as u64 != meta.size { bail!("errors.localImageInvalid"); }
            Ok((meta, content))
        }
        _ => bail!("errors.unexpectedPacketSync"),
    }
}

fn is_crdt_state(path: &str) -> bool {
    path.starts_with(".lownotes/crdt/") && path.ends_with(".bin")
}

fn skip_markdown_sync(path: &str, local: &Manifest, remote: &Manifest) -> bool {
    // Modern peers exchange the authority rather than importing its projection
    // as extra legacy additions. Older peers can still read the plain list.
    if path == crate::links::LINKS_REL_PATH
        && local.contains_key(crate::link_operations::RELATIVE_PATH)
        && remote.contains_key(crate::link_operations::RELATIVE_PATH) { return true; }
    if !vault::is_markdown(Path::new(path)) { return false; }
    let state_path = CrdtManager::state_relative_path(path);
    local.contains_key(&state_path) || remote.contains_key(&state_path)
}

fn read_sync_content(vault_path: &Path, path: &str) -> anyhow::Result<Vec<u8>> {
    if crate::links::is_sync_metadata(path) {
        Ok(std::fs::read(vault::safe_join(vault_path, path)?)?)
    } else if crate::local_images::is_sync_image(path) {
        Ok(crate::local_images::load(vault_path, crate::local_images::sync_name(path)?)?.bytes)
    } else if is_crdt_state(path) {
        Ok(std::fs::read(vault::safe_join(vault_path, path)?)?)
    } else {
        Ok(vault::read_note(vault_path, path)?.into_bytes())
    }
}

struct WriteOutcome {
    changed: bool,
    conflict_created: bool,
}

fn write_sync_content(vault_path: &Path, path: &str, content: &[u8], app: Option<&AppHandle>) -> anyhow::Result<WriteOutcome> {
    if crate::links::is_sync_metadata(path) {
        Ok(WriteOutcome { changed: crate::links::merge_sync(vault_path, path, content)?, conflict_created: false })
    } else if crate::local_images::is_sync_image(path) {
        let changed = crate::local_images::insert(vault_path, crate::local_images::sync_name(path)?, content)?;
        Ok(WriteOutcome { changed, conflict_created: false })
    } else if is_crdt_state(path) {
        let manager = app.map(|app| app.state::<AppState>().crdt.clone()).unwrap_or_default();
        let (note_path, result) = manager.merge_state_file(vault_path, path, content)?;
        if let (Some(app), Some(conflict_path)) = (app, result.conflict_path.as_ref()) {
            let _ = app.emit("p2p:conflict", NetworkEventPayload::Conflict {
                note_path: note_path.clone(),
                conflict_path: conflict_path.clone(),
            });
        }
        if result.changed {
            if let Some(app) = app {
                let _ = app.emit("p2p:crdt-update", NetworkEventPayload::RemoteCrdtUpdate {
                    note_path, update: result.state,
                });
            }
        }
        Ok(WriteOutcome { changed: result.changed, conflict_created: result.conflict_path.is_some() })
    } else {
        vault::save_note(vault_path, path, std::str::from_utf8(content)?)?;
        Ok(WriteOutcome { changed: true, conflict_created: false })
    }
}

async fn send_crdt_to_peer(
    endpoint: Endpoint,
    peer: PeerConfig,
    note_path: String,
    update: Vec<u8>,
) -> anyhow::Result<()> {
    let addr = peer.endpoint_addr()?;
    let connection = endpoint.connect(addr, LEGACY_ALPN).await?;
    let (mut send, _) = connection.open_bi().await?;
    send_packet(&mut send, &Packet::CrdtUpdate { note_path, update }).await?;
    send.finish()?;
    let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
    connection.close(0u32.into(), b"crdt sent");
    Ok(())
}

async fn send_awareness_to_peer(
    endpoint: Endpoint,
    peer: PeerConfig,
    note_path: String,
    update: Vec<u8>,
) -> anyhow::Result<()> {
    let addr = peer.endpoint_addr()?;
    let connection = endpoint.connect(addr, LEGACY_ALPN).await?;
    let (mut send, _) = connection.open_bi().await?;
    send_packet(&mut send, &Packet::Awareness { note_path, update }).await?;
    send.finish()?;
    let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
    connection.close(0u32.into(), b"awareness sent");
    Ok(())
}

async fn send_delete_to_peer(
    endpoint: Endpoint,
    peer: PeerConfig,
    path: String,
) -> anyhow::Result<()> {
    let addr = peer.endpoint_addr()?;
    let connection = endpoint.connect(addr, LEGACY_ALPN).await?;
    let (mut send, _) = connection.open_bi().await?;
    send_packet(&mut send, &Packet::Delete { path }).await?;
    send.finish()?;
    let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
    connection.close(0u32.into(), b"delete sent");
    Ok(())
}

fn connection_is_direct(connection: &iroh::endpoint::Connection) -> Option<bool> {
    let paths = connection.paths();
    let selected = paths.iter().find(|path| path.is_selected())?;
    Some(selected.is_ip())
}

async fn send_packet(stream: &mut iroh::endpoint::SendStream, packet: &Packet) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(packet)?;
    if bytes.len() > MAX_PACKET_BYTES {
        bail!("errors.packetTooLarge");
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn recv_packet<T: DeserializeOwned>(stream: &mut iroh::endpoint::RecvStream) -> anyhow::Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PACKET_BYTES {
        bail!("errors.packetReceivedTooLarge");
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use yrs::{Text, Transact, updates::decoder::Decode};

    fn temp_vault(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lownotes-sync-test-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn peer_of(endpoint: &Endpoint, name: &str) -> PeerConfig {
        PeerConfig {
            name: name.to_string(),
            endpoint_id: endpoint.id().to_string(),
            ticket: EndpointTicket::new(endpoint.addr()).to_string(),
        }
    }

    async fn bind_endpoint() -> Endpoint {
        Endpoint::builder(presets::N0)
            .secret_key(SecretKey::generate())
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_sync_exchanges_notes_between_vaults() {
        let vault_a = temp_vault("a");
        let vault_b = temp_vault("b");
        fs::write(vault_a.join("nota_a.md"), "# Nota A\n\nConteudo de A.").unwrap();
        fs::write(vault_b.join("nota_b.md"), "# Nota B\n\nConteudo de B.").unwrap();
        fs::create_dir_all(vault_a.join(".lownotes")).unwrap();
        fs::write(
            vault_a.join(".lownotes/links.json"),
            r#"{"version":1,"links":[{"source":"nota_a.md","target":"nota_b.md","origin":"manual"}]}"#,
        )
        .unwrap();

        let ep_a = bind_endpoint().await;
        let ep_b = bind_endpoint().await;
        let peer_a = peer_of(&ep_a, "Vault Casa");
        let peer_b = peer_of(&ep_b, "Notas Trabalho");

        let (accept_ep, accept_vault) = (ep_a.clone(), vault_a.clone());
        let responder = tokio::spawn(async move {
            for _ in 0..4 {
                let incoming = ep_accept(&accept_ep).await;
                let connection = incoming.await.unwrap();
                let (mut send, mut recv) = connection.accept_bi().await.unwrap();
                let packet: Packet = recv_packet(&mut recv).await.unwrap();
                match packet {
                    Packet::Manifest(remote_manifest) => {
                        serve_sync(&mut send, &mut recv, &accept_vault, remote_manifest, None, true, true)
                            .await
                            .unwrap();
                    }
                    other => panic!("pacote inesperado: {other:?}"),
                }
                connection.close(0u32.into(), b"sync complete");
            }
        });

        let (changed_b, _direct, _) = dial_sync(ep_b.clone(), vault_b.clone(), peer_a.clone(), None)
            .await
            .unwrap();
        assert_eq!(changed_b, 2, "B recebe uma nota e a relação; a projeção repetida não é uma nova alteração");
        assert!(vault_b.join("nota_a.md").exists(), "vault B nao recebeu nota_a.md");
        assert_eq!(
            fs::read_to_string(vault_b.join("nota_a.md")).unwrap(),
            "# Nota A\n\nConteudo de A."
        );
        assert!(vault_a.join("nota_b.md").exists(), "vault A nao recebeu nota_b.md");
        assert_eq!(
            fs::read_to_string(vault_a.join("nota_b.md")).unwrap(),
            "# Nota B\n\nConteudo de B."
        );
        // Hidden link store must travel with the sync (backup/restore guarantee)
        assert!(
            vault_b.join(".lownotes/links.json").exists(),
            "vault B nao recebeu .lownotes/links.json"
        );

        // Incremental: edit on B, second sync must converge A without ping-pong
        // Make the newer timestamp explicit: Windows can give two rapid writes
        // the same millisecond, which is a different reconciliation scenario.
        let previous_time = fs::metadata(vault_a.join("nota_b.md")).unwrap().modified().unwrap();
        fs::write(vault_b.join("nota_b.md"), "# Nota B\n\nConteudo de B atualizado.").unwrap();
        filetime::set_file_mtime(vault_b.join("nota_b.md"),
            filetime::FileTime::from_system_time(previous_time + Duration::from_secs(1))).unwrap();
        let (changed_b2, _, _) = dial_sync(ep_b.clone(), vault_b.clone(), peer_a.clone(), None)
            .await
            .unwrap();
        assert_eq!(changed_b2, 0, "B nao deveria receber nada de volta");
        assert_eq!(
            fs::read_to_string(vault_a.join("nota_b.md")).unwrap(),
            "# Nota B\n\nConteudo de B atualizado."
        );
        assert_eq!(
            fs::read_to_string(vault_b.join("nota_a.md")).unwrap(),
            "# Nota A\n\nConteudo de A."
        );

        // Offline edits to the same note keep both complete versions rather
        // than interleaving their text into one line.
        let crdt_a = CrdtManager::new();
        let crdt_b = CrdtManager::new();
        let baseline_a = crdt_a.get_or_create_doc(&vault_a, "nota_b.md").unwrap();
        let baseline_b = crdt_b.get_or_create_doc(&vault_b, "nota_b.md").unwrap();
        assert_eq!(baseline_a, baseline_b);
        let editor_a = yrs::Doc::with_client_id(3001);
        editor_a.transact_mut().apply_update(yrs::Update::decode_v1(&baseline_a).unwrap()).unwrap();
        editor_a.get_or_insert_text("content").push(&mut editor_a.transact_mut(), " Alice");
        let editor_b = yrs::Doc::with_client_id(3002);
        editor_b.transact_mut().apply_update(yrs::Update::decode_v1(&baseline_b).unwrap()).unwrap();
        editor_b.get_or_insert_text("content").push(&mut editor_b.transact_mut(), " Bob");
        crdt_a.apply_update(&vault_a, "nota_b.md", &CrdtManager::encode_state(&editor_a)).unwrap();
        crdt_b.apply_update(&vault_b, "nota_b.md", &CrdtManager::encode_state(&editor_b)).unwrap();
        let (merged, _, conflict_created) = dial_sync(ep_b.clone(), vault_b.clone(), peer_a.clone(), None).await.unwrap();
        assert_eq!(merged, 1);
        assert!(conflict_created);
        let text_a = fs::read_to_string(vault_a.join("nota_b.md")).unwrap();
        let text_b = fs::read_to_string(vault_b.join("nota_b.md")).unwrap();
        assert_eq!(text_a, text_b);
        assert!(text_a.ends_with(" Alice") || text_a.ends_with(" Bob"));
        assert!(!text_a.contains(" Alice Bob") && !text_a.contains(" Bob Alice"));
        let conflict_notes: Vec<_> = vault::list_vault_items(&vault_b).unwrap().into_iter()
            .filter(|item| item.path.contains("(conflict ")).collect();
        assert_eq!(conflict_notes.len(), 1);
        let other = vault::read_note(&vault_b, &conflict_notes[0].path).unwrap();
        assert_ne!(text_b, other);
        assert!(other.ends_with(" Alice") || other.ends_with(" Bob"));
        let (_, _, second_conflict) = dial_sync(ep_b.clone(), vault_b.clone(), peer_a.clone(), None).await.unwrap();
        assert!(!second_conflict);
        responder.await.unwrap();
        assert_eq!(vault::read_note(&vault_a, &conflict_notes[0].path).unwrap(), other);

        let _ = peer_b;
        let _ = fs::remove_dir_all(&vault_a);
        let _ = fs::remove_dir_all(&vault_b);
    }

    #[tokio::test]
    async fn images_created_offline_on_both_devices_sync_incrementally_without_database_conflicts() {
        use image::ImageEncoder;
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let make_png = |color| {
            let image = image::RgbaImage::from_pixel(16, 16, image::Rgba(color));
            let mut bytes = Vec::new();
            image::codecs::png::PngEncoder::new(&mut bytes).write_image(&image, 16, 16, image::ExtendedColorType::Rgba8).unwrap();
            bytes
        };
        let link_a = crate::local_images::save_pasted(a.path(), make_png([255, 0, 0, 255])).unwrap();
        let link_b = crate::local_images::save_pasted(b.path(), make_png([0, 255, 0, 128])).unwrap();
        fs::write(a.path().join("a.md"), format!("![red]({link_a})")).unwrap();
        fs::write(b.path().join("b.md"), format!("![green]({link_b})")).unwrap();
        let ep_a = bind_endpoint().await;
        let ep_b = bind_endpoint().await;
        let peer_a = peer_of(&ep_a, "A");
        let server_ep = ep_a.clone();
        let server_root = a.path().to_path_buf();
        let responder = tokio::spawn(async move {
            for _ in 0..2 {
                let connection = ep_accept(&server_ep).await.await.unwrap();
                let (mut send, mut recv) = connection.accept_bi().await.unwrap();
                let Packet::Manifest(manifest) = recv_packet(&mut recv).await.unwrap() else { panic!("Expected manifest"); };
                serve_sync(&mut send, &mut recv, &server_root, manifest, None, true, true).await.unwrap();
            }
        });
        let (changed, _, _) = dial_sync(ep_b.clone(), b.path().to_path_buf(), peer_a.clone(), None).await.unwrap();
        assert_eq!(changed, 2, "one new note and one new image");
        assert_eq!(crate::local_images::manifest(a.path()).unwrap().len(), 2);
        assert_eq!(crate::local_images::manifest(b.path()).unwrap().len(), 2);
        for link in [&link_a, &link_b] {
            let name = crate::local_images::link_name(link).unwrap();
            assert_eq!(crate::local_images::load(a.path(), name).unwrap().bytes, crate::local_images::load(b.path(), name).unwrap().bytes);
        }
        assert_eq!(dial_sync(ep_b.clone(), b.path().to_path_buf(), peer_a, None).await.unwrap().0, 0);
        responder.await.unwrap();
        ep_a.close().await;
        ep_b.close().await;
    }

    #[tokio::test]
    async fn legacy_sync_still_exchanges_notes_without_sending_image_packets() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("a.md"), "old-compatible note").unwrap();
        let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC").unwrap();
        crate::local_images::save_pasted(a.path(), bytes).unwrap();
        let ep_a = Endpoint::builder(presets::N0).secret_key(SecretKey::generate()).alpns(vec![LEGACY_ALPN.to_vec()]).bind().await.unwrap();
        let ep_b = bind_endpoint().await;
        let peer_a = peer_of(&ep_a, "Old device");
        let root = a.path().to_path_buf();
        let server_ep = ep_a.clone();
        let responder = tokio::spawn(async move {
            // Ignore incompatible v4/v3 handshakes, then accept the v2 fallback.
            loop {
                let incoming = ep_accept(&server_ep).await;
                let Ok(connection) = incoming.await else { continue; };
                let (mut send, mut recv) = connection.accept_bi().await.unwrap();
                let Packet::Manifest(manifest) = recv_packet(&mut recv).await.unwrap() else { panic!("Expected manifest"); };
                assert!(!manifest.keys().any(|p| crate::local_images::is_sync_image(p)));
                serve_sync(&mut send, &mut recv, &root, manifest, None, false, false).await.unwrap();
                break;
            }
        });
        let result = tokio::time::timeout(Duration::from_secs(30), dial_sync(ep_b.clone(), b.path().to_path_buf(), peer_a, None)).await.unwrap().unwrap();
        assert_eq!(result.0, 1);
        assert_eq!(fs::read_to_string(b.path().join("a.md")).unwrap(), "old-compatible note");
        assert!(crate::local_images::manifest(b.path()).unwrap().is_empty());
        responder.await.unwrap();
        ep_a.close().await;
        ep_b.close().await;
    }

    #[test]
    fn binary_image_packets_fit_the_limit_and_reject_inconsistent_metadata() {
        let content = vec![255; crate::local_images::MAX_STORED_BYTES];
        let id = blake3::hash(&content).to_hex().to_string();
        let meta = NoteMeta { path: format!(".lownotes/images/{id}.png"), hash: id, size: content.len() as u64, modified_ms: 0 };
        let packet = Packet::ImagePut { meta: meta.clone(), content_base64: STANDARD.encode(&content) };
        assert!(serde_json::to_vec(&packet).unwrap().len() < MAX_PACKET_BYTES);
        assert!(unpack_sync_content(packet, false, false).is_err());
        let packet = Packet::ImagePut { meta, content_base64: STANDARD.encode(b"different data") };
        assert!(unpack_sync_content(packet, true, true).is_err());
    }

    #[test]
    fn json_metadata_avoids_numeric_byte_expansion_and_is_rejected_on_older_protocols() {
        let mut history = crate::link_operations::LinkChanges::default();
        for index in 0..1800 {
            history.additions.insert(format!("add-{index:032x}"), crate::links::LinkEdge {
                source: format!("{}-{index}.md", "a".repeat(4000)),
                target: format!("{}.md", "b".repeat(4000)),
                origin: crate::links::LinkOrigin::manual,
            });
        }
        let bytes = history.encode().unwrap();
        assert!(bytes.len() > 10 * 1024 * 1024);
        let meta = NoteMeta { path: crate::link_operations::RELATIVE_PATH.into(),
            hash: blake3::hash(&bytes).to_hex().to_string(), size: bytes.len() as u64, modified_ms: 0 };
        let packet = Packet::JsonPut { meta: meta.clone(), content: serde_json::from_slice(&bytes).unwrap() };
        assert!(serde_json::to_vec(&packet).unwrap().len() < MAX_PACKET_BYTES);
        let (_, received) = unpack_sync_content(packet, true, true).unwrap();
        assert_eq!(crate::link_operations::LinkChanges::decode(&received).unwrap(), history);
        let packet = Packet::JsonPut { meta, content: serde_json::from_slice(&bytes).unwrap() };
        assert!(unpack_sync_content(packet, true, false).is_err());
    }

    #[tokio::test]
    async fn v3_fallback_keeps_images_and_legacy_links_without_sending_operation_packets() {
        let a = tempfile::tempdir().unwrap(); let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("a.md"), "A").unwrap(); fs::write(a.path().join("b.md"), "B").unwrap();
        crate::links::apply_operations(a.path(), &[crate::links::LinkOperation {
            source: "a.md".into(), target: "b.md".into(), action: crate::links::LinkAction::add,
        }], crate::links::LinkOrigin::manual).unwrap();
        let image = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC").unwrap();
        let link = crate::local_images::save_pasted(a.path(), image).unwrap();
        let server = Endpoint::builder(presets::N0).secret_key(SecretKey::generate()).alpns(vec![IMAGE_ALPN.to_vec()]).bind().await.unwrap();
        let client = bind_endpoint().await; let peer = peer_of(&server, "v3 peer");
        let accept_ep = server.clone(); let root = a.path().to_path_buf();
        let responder = tokio::spawn(async move {
            loop {
                let Ok(connection) = ep_accept(&accept_ep).await.await else { continue; };
                let (mut send, mut recv) = connection.accept_bi().await.unwrap();
                let Packet::Manifest(manifest) = recv_packet(&mut recv).await.unwrap() else { panic!("Expected manifest"); };
                assert!(!manifest.contains_key(crate::link_operations::RELATIVE_PATH));
                serve_sync(&mut send, &mut recv, &root, manifest, None, true, false).await.unwrap();
                break;
            }
        });
        tokio::time::timeout(Duration::from_secs(30), dial_sync(client.clone(), b.path().into(), peer, None)).await.unwrap().unwrap();
        responder.await.unwrap();
        assert_eq!(crate::links::graph_links(b.path()).unwrap().len(), 1);
        let name = crate::local_images::link_name(&link).unwrap();
        assert_eq!(crate::local_images::load(a.path(), name).unwrap().bytes, crate::local_images::load(b.path(), name).unwrap().bytes);
        server.close().await; client.close().await;
    }

    async fn sync_test_pair(client: &Endpoint, client_root: &Path, server: &Endpoint, server_root: &Path) -> usize {
        let server_ep = server.clone();
        let root = server_root.to_path_buf();
        let peer = peer_of(server, "test replica");
        let responder = tokio::spawn(async move {
            let connection = ep_accept(&server_ep).await.await.unwrap();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            let Packet::Manifest(manifest) = recv_packet(&mut recv).await.unwrap() else { panic!("Expected manifest"); };
            serve_sync(&mut send, &mut recv, &root, manifest, None, true, true).await.unwrap();
        });
        let outcome = tokio::time::timeout(Duration::from_secs(30),
            dial_sync(client.clone(), client_root.to_path_buf(), peer, None)).await.unwrap().unwrap();
        responder.await.unwrap();
        outcome.0
    }

    #[tokio::test]
    async fn three_devices_merge_offline_map_changes_without_replaying_a_deleted_link() {
        use crate::links::{self, LinkAction, LinkOperation, LinkOrigin};
        let roots = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
        for root in &roots {
            for name in ["a.md", "b.md", "c.md", "d.md"] { fs::write(root.path().join(name), format!("# {name}\n")).unwrap(); }
        }
        let endpoints = [bind_endpoint().await, bind_endpoint().await, bind_endpoint().await];
        let op = |target: &str, action| LinkOperation { source: "a.md".into(), target: target.into(), action };
        links::apply_operations(roots[0].path(), &[op("b.md", LinkAction::add)], LinkOrigin::manual).unwrap();
        sync_test_pair(&endpoints[1], roots[1].path(), &endpoints[0], roots[0].path()).await;
        sync_test_pair(&endpoints[2], roots[2].path(), &endpoints[0], roots[0].path()).await;
        let stale_operations = fs::read(roots[0].path().join(crate::link_operations::RELATIVE_PATH)).unwrap();
        let stale_projection = fs::read(roots[0].path().join(links::LINKS_REL_PATH)).unwrap();
        // Separate offline operations, deliberately unrelated file timestamps.
        links::apply_operations(roots[0].path(), &[op("b.md", LinkAction::remove)], LinkOrigin::manual).unwrap();
        links::apply_operations(roots[1].path(), &[op("c.md", LinkAction::add)], LinkOrigin::agent).unwrap();
        links::apply_operations(roots[2].path(), &[op("d.md", LinkAction::add)], LinkOrigin::manual).unwrap();
        for (index, root) in roots.iter().enumerate() {
            filetime::set_file_mtime(root.path().join(crate::link_operations::RELATIVE_PATH),
                filetime::FileTime::from_unix_time(100 - index as i64, 0)).unwrap();
        }
        for _ in 0..2 {
            sync_test_pair(&endpoints[1], roots[1].path(), &endpoints[0], roots[0].path()).await;
            sync_test_pair(&endpoints[2], roots[2].path(), &endpoints[1], roots[1].path()).await;
            sync_test_pair(&endpoints[0], roots[0].path(), &endpoints[2], roots[2].path()).await;
        }
        let expected = links::graph_links(roots[0].path()).unwrap();
        assert_eq!(expected.len(), 2);
        assert!(expected.iter().any(|edge| edge.target == "c.md" && edge.origin == LinkOrigin::agent));
        assert!(expected.iter().any(|edge| edge.target == "d.md" && edge.origin == LinkOrigin::manual));
        for root in &roots {
            assert_eq!(links::graph_links(root.path()).unwrap(), expected);
            assert!(!write_sync_content(root.path(), crate::link_operations::RELATIVE_PATH, &stale_operations, None).unwrap().changed);
            write_sync_content(root.path(), links::LINKS_REL_PATH, &stale_projection, None).unwrap();
            assert_eq!(links::graph_links(root.path()).unwrap(), expected);
        }
        // After restart, no cached in-memory state is needed to prevent resurrection.
        assert_eq!(sync_test_pair(&endpoints[1], roots[1].path(), &endpoints[0], roots[0].path()).await, 0);
        for endpoint in endpoints { endpoint.close().await; }
    }

    async fn ep_accept(endpoint: &Endpoint) -> iroh::endpoint::Incoming {
        endpoint.accept().await.expect("endpoint fechado")
    }
}
