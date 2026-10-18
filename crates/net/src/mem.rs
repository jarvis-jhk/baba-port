//! An in-process stand-in for a Warpzone server: rooms (public / unlisted,
//! open / code / approve), codes, invite links, knocks, the room list,
//! leaving, kicking, crashing, and star-shaped peer channels (a guest talks
//! to the host only), with the same [`Event`]s as [`warpzone_core::client`].
//! For unit tests and for running several machines in one process.

use crate::link::Link;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::time::Duration;
use warpzone_core::client::{Event, HostOptions};
use warpzone_core::net::PeerId;
use warpzone_core::protocol::api::{Account, Entry, RoomInfo, Visibility};

struct MemNode {
    name: String,
    account: i64,
    alive: bool,
    room: Option<String>,
    events: VecDeque<Event>,
}

struct MemRoom {
    id: String,
    code: Option<String>,
    secret: String,
    host: usize,
    members: Vec<usize>,
    opts: HostOptions,
    closed: bool,
    knocks: BTreeMap<String, usize>,
    approved: BTreeSet<usize>,
    created: i64,
}

#[derive(Default)]
struct Inner {
    game: String,
    nodes: Vec<MemNode>,
    rooms: BTreeMap<String, MemRoom>,
    next: u32,
}

/// The in-process "server". Clone it to share.
#[derive(Clone, Default)]
pub struct MemNet(Rc<RefCell<Inner>>);

/// One machine's connection to a [`MemNet`].
pub struct MemClient {
    net: MemNet,
    id: usize,
}

// A node as others see it.
fn pid(node: usize) -> PeerId {
    node as PeerId + 1
}

fn node_of(peer: PeerId) -> usize {
    peer as usize - 1
}

impl Inner {
    fn info(&self, r: &MemRoom) -> RoomInfo {
        RoomInfo {
            id: r.id.clone(),
            game: self.game.clone(),
            game_version: "mem".into(),
            name: format!("{}'s room", self.nodes[r.host].name),
            host: self.nodes[r.host].name.clone(),
            players: r.members.len() as u32,
            max_players: r.opts.max_players,
            visibility: r.opts.visibility,
            entry: r.opts.entry,
            audience: r.opts.audience.clone(),
            ranked: false,
            staged: false,
            lockfile: None,
            preset: r.opts.preset.clone(),
            metadata: r.opts.metadata.clone(),
            members: r
                .members
                .iter()
                .map(|&m| self.nodes[m].name.clone())
                .collect(),
            created: r.created,
        }
    }

    fn push(&mut self, node: usize, e: Event) {
        if self.nodes[node].alive {
            self.nodes[node].events.push_back(e);
        }
    }

    fn error(&mut self, node: usize, op: &str, code: &str, message: &str) {
        self.push(
            node,
            Event::Error {
                op: op.into(),
                code: code.into(),
                message: message.into(),
            },
        );
    }

    fn add_member(&mut self, room: &str, node: usize) {
        let host = {
            let r = self.rooms.get_mut(room).unwrap();
            r.members.push(node);
            r.host
        };
        self.nodes[node].room = Some(room.into());
        let info = self.info(&self.rooms[room]);
        self.push(
            node,
            Event::RoomJoined {
                room: info,
                host: false,
            },
        );
        let (hn, ha) = (self.nodes[host].name.clone(), self.nodes[host].account);
        self.push(
            node,
            Event::PeerJoined {
                peer: pid(host),
                account: ha,
                username: hn,
            },
        );
        self.push(
            node,
            Event::PeerConnected {
                peer: pid(host),
                relayed: false,
            },
        );
        let (gn, ga) = (self.nodes[node].name.clone(), self.nodes[node].account);
        self.push(
            host,
            Event::PeerJoined {
                peer: pid(node),
                account: ga,
                username: gn,
            },
        );
        self.push(
            host,
            Event::PeerConnected {
                peer: pid(node),
                relayed: false,
            },
        );
    }

    fn try_join(&mut self, node: usize, room: &str, code: Option<&str>, secret: Option<&str>) {
        self.leave(node);
        let Some(r) = self.rooms.get(room) else {
            return self.error(node, "join", "not_found", "no such room");
        };
        let code_ok = code.is_some() && code.map(str::to_ascii_uppercase) == r.code;
        let invite_ok = secret == Some(r.secret.as_str());
        let approved = r.approved.contains(&node);
        if r.opts.visibility == Visibility::Unlisted && !(code_ok || invite_ok || approved) {
            return self.error(node, "join", "not_found", "no such room");
        }
        if r.closed && !approved {
            return self.error(node, "join", "forbidden", "this game has already started");
        }
        if r.members.len() as u32 >= r.opts.max_players {
            return self.error(node, "join", "conflict", "the room is full");
        }
        let allowed = invite_ok
            || approved
            || match r.opts.entry {
                Entry::Open => true,
                Entry::Code => code_ok,
                Entry::Approve => false,
            };
        if allowed {
            return self.add_member(room, node);
        }
        if r.opts.entry == Entry::Code {
            return self.error(node, "join", "forbidden", "wrong join code");
        }
        self.next += 1;
        let knock = format!("k{}", self.next);
        let host = r.host;
        let info = self.info(r);
        self.rooms
            .get_mut(room)
            .unwrap()
            .knocks
            .insert(knock.clone(), node);
        self.push(node, Event::Knocking { room: info });
        let (name, account) = (self.nodes[node].name.clone(), self.nodes[node].account);
        self.push(
            host,
            Event::Knock {
                knock,
                account,
                username: name,
            },
        );
    }

    /// Leave the current room (gracefully).
    fn leave(&mut self, node: usize) {
        let Some(room) = self.nodes[node].room.take() else {
            return;
        };
        let Some(r) = self.rooms.get_mut(&room) else {
            return;
        };
        if r.host == node {
            let r = self.rooms.remove(&room).unwrap();
            for m in r.members.into_iter().filter(|&m| m != node) {
                self.nodes[m].room = None;
                self.push(m, Event::RoomClosed { room: room.clone() });
            }
        } else {
            r.members.retain(|&m| m != node);
            let host = r.host;
            let account = self.nodes[node].account;
            self.push(
                host,
                Event::PeerLeft {
                    peer: pid(node),
                    account,
                },
            );
        }
    }
}

impl MemNet {
    /// A server for game id `game`.
    pub fn new(game: &str) -> MemNet {
        let n = MemNet::default();
        n.0.borrow_mut().game = game.into();
        n
    }

    /// A new machine with its own account (logged in at once).
    pub fn client(&self, name: &str) -> MemClient {
        let mut h = self.0.borrow_mut();
        let id = h.nodes.len();
        let account = id as i64 + 1;
        h.nodes.push(MemNode {
            name: name.into(),
            account,
            alive: true,
            room: None,
            events: VecDeque::new(),
        });
        h.push(
            id,
            Event::LoggedIn {
                account: Some(Account {
                    id: account,
                    username: name.into(),
                    created: 0,
                }),
                roles: Vec::new(),
            },
        );
        MemClient {
            net: self.clone(),
            id,
        }
    }

    /// Ids of every open room.
    pub fn rooms(&self) -> Vec<String> {
        self.0.borrow().rooms.keys().cloned().collect()
    }
}

impl MemClient {
    /// The process dies: no goodbye. Guests of a crashed host see the host
    /// time out (`PeerLeft`, then `RoomClosed`); a crashed guest times out
    /// at the host (`PeerLeft`).
    pub fn crash(&mut self) {
        let mut h = self.net.0.borrow_mut();
        if !h.nodes[self.id].alive {
            return;
        }
        h.nodes[self.id].alive = false;
        h.nodes[self.id].events.clear();
        let Some(room) = h.nodes[self.id].room.take() else {
            return;
        };
        let account = h.nodes[self.id].account;
        let Some(r) = h.rooms.get_mut(&room) else {
            return;
        };
        if r.host == self.id {
            let r = h.rooms.remove(&room).unwrap();
            for m in r.members.into_iter().filter(|&m| m != self.id) {
                h.nodes[m].room = None;
                h.push(
                    m,
                    Event::PeerLeft {
                        peer: pid(self.id),
                        account,
                    },
                );
                h.push(m, Event::RoomClosed { room: room.clone() });
            }
        } else {
            r.members.retain(|&m| m != self.id);
            let host = r.host;
            h.push(
                host,
                Event::PeerLeft {
                    peer: pid(self.id),
                    account,
                },
            );
        }
    }

    pub fn alive(&self) -> bool {
        self.net.0.borrow().nodes[self.id].alive
    }
}

impl Drop for MemClient {
    fn drop(&mut self) {
        let mut h = self.net.0.borrow_mut();
        if h.nodes[self.id].alive {
            h.leave(self.id);
            h.nodes[self.id].alive = false;
        }
    }
}

impl Link for MemClient {
    fn poll(&mut self) {}

    fn next_event(&mut self) -> Option<Event> {
        self.net.0.borrow_mut().nodes[self.id].events.pop_front()
    }

    fn send(&mut self, peer: PeerId, channel: u8, data: &[u8], reliable: bool) -> bool {
        let mut h = self.net.0.borrow_mut();
        let to = node_of(peer);
        if to >= h.nodes.len() || to == self.id || !h.nodes[self.id].alive || !h.nodes[to].alive {
            return false;
        }
        let (Some(a), Some(b)) = (&h.nodes[self.id].room, &h.nodes[to].room) else {
            return false;
        };
        if a != b {
            return false;
        }
        let host = h.rooms[a].host;
        if host != self.id && host != to {
            return false;
        }
        let me = self.id;
        h.push(
            to,
            Event::PeerData {
                peer: pid(me),
                channel,
                data: data.to_vec(),
                reliable,
            },
        );
        true
    }

    fn register(&mut self, _username: &str) {}

    fn host(&mut self, mut opts: HostOptions) {
        let mut h = self.net.0.borrow_mut();
        h.leave(self.id);
        if opts.max_players < 2 {
            opts.max_players = 16;
        }
        h.next += 1;
        let n = h.next;
        let id = format!("mem{n}");
        let code = (opts.visibility == Visibility::Unlisted || opts.entry == Entry::Code)
            .then(|| format!("C{n:05}"));
        let room = MemRoom {
            id: id.clone(),
            code: code.clone(),
            secret: format!("s{n}"),
            host: self.id,
            members: vec![self.id],
            opts,
            closed: false,
            knocks: BTreeMap::new(),
            approved: BTreeSet::new(),
            created: n as i64,
        };
        let invite = format!("warpzone://join/{id}#{}", room.secret);
        let info = h.info(&room);
        h.rooms.insert(id.clone(), room);
        h.nodes[self.id].room = Some(id);
        h.push(
            self.id,
            Event::RoomHosted {
                room: info.clone(),
                code,
                invite,
            },
        );
        h.push(
            self.id,
            Event::RoomJoined {
                room: info,
                host: true,
            },
        );
    }

    fn join(&mut self, room: &str, code: Option<&str>, secret: Option<&str>) {
        self.net
            .0
            .borrow_mut()
            .try_join(self.id, room, code, secret);
    }

    fn join_code(&mut self, code: &str) {
        let mut h = self.net.0.borrow_mut();
        let code = code.trim().to_ascii_uppercase();
        let room = h
            .rooms
            .values()
            .find(|r| r.code.as_deref() == Some(code.as_str()))
            .map(|r| r.id.clone());
        match room {
            Some(r) => h.try_join(self.id, &r, Some(&code), None),
            None => h.error(self.id, "join", "not_found", "no room with that code"),
        }
    }

    fn list_rooms(&mut self, has_space: bool) {
        let mut h = self.net.0.borrow_mut();
        let mut list: Vec<RoomInfo> = h
            .rooms
            .values()
            .filter(|r| r.opts.visibility == Visibility::Public)
            .filter(|r| !has_space || ((r.members.len() as u32) < r.opts.max_players && !r.closed))
            .map(|r| h.info(r))
            .collect();
        list.sort_by_key(|r| std::cmp::Reverse(r.created));
        h.push(self.id, Event::RoomList(list));
    }

    fn answer_knock(&mut self, knock: &str, accept: bool) {
        let mut h = self.net.0.borrow_mut();
        let Some(room) = h.nodes[self.id].room.clone() else {
            return;
        };
        let Some(r) = h.rooms.get_mut(&room) else {
            return;
        };
        if r.host != self.id {
            return;
        }
        let Some(node) = r.knocks.remove(knock) else {
            return;
        };
        if accept {
            r.approved.insert(node);
            if h.nodes[node].alive && h.nodes[node].room.is_none() {
                h.add_member(&room, node);
            }
        } else {
            h.push(node, Event::KnockDenied { room });
        }
    }

    fn kick(&mut self, account: i64, _ban: bool) {
        let mut h = self.net.0.borrow_mut();
        let Some(room) = h.nodes[self.id].room.clone() else {
            return;
        };
        let Some(r) = h.rooms.get(&room) else {
            return;
        };
        if r.host != self.id {
            return;
        }
        let Some(node) = r
            .members
            .iter()
            .copied()
            .find(|&m| m != self.id && h.nodes[m].account == account)
        else {
            return;
        };
        h.rooms
            .get_mut(&room)
            .unwrap()
            .members
            .retain(|&m| m != node);
        h.nodes[node].room = None;
        h.push(
            node,
            Event::Kicked {
                room: room.clone(),
                banned: false,
            },
        );
        h.push(
            self.id,
            Event::PeerLeft {
                peer: pid(node),
                account,
            },
        );
    }

    fn set_closed(&mut self, closed: bool) {
        let mut h = self.net.0.borrow_mut();
        if let Some(room) = h.nodes[self.id].room.clone()
            && let Some(r) = h.rooms.get_mut(&room)
            && r.host == self.id
        {
            r.closed = closed;
        }
    }

    fn set_metadata(&mut self, metadata: BTreeMap<String, String>) {
        let mut h = self.net.0.borrow_mut();
        if let Some(room) = h.nodes[self.id].room.clone()
            && let Some(r) = h.rooms.get_mut(&room)
            && r.host == self.id
        {
            r.opts.metadata = metadata;
        }
    }

    fn leave(&mut self) {
        self.net.0.borrow_mut().leave(self.id);
    }

    fn username(&self) -> Option<String> {
        Some(self.net.0.borrow().nodes[self.id].name.clone())
    }

    fn account_id(&self) -> i64 {
        self.net.0.borrow().nodes[self.id].account
    }

    fn peer_rtt(&self, _peer: PeerId) -> Option<Duration> {
        Some(Duration::ZERO)
    }

    fn peer_relayed(&self, _peer: PeerId) -> Option<bool> {
        Some(false)
    }

    fn force_relay(&mut self, _on: bool) {}

    fn crash(&mut self) {
        MemClient::crash(self)
    }
}
