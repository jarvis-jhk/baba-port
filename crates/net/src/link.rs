//! Where peer traffic comes from: the [`Link`] trait over a Warpzone
//! [`Client`] or an in-process [`crate::mem::MemNet`], and the link
//! conditioner ([`LinkSim`]) that adds latency, jitter and loss for tests.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};
use warpzone_core::client::{Client, Event, HostOptions};
use warpzone_core::net::PeerId;

/// The part of a Warpzone client a session uses. Implemented by
/// [`Client`] and [`crate::mem::MemClient`]; every request answers with
/// [`Event`]s exactly as Warpzone documents them.
pub trait Link {
    fn poll(&mut self);
    fn next_event(&mut self) -> Option<Event>;
    fn send(&mut self, peer: PeerId, channel: u8, data: &[u8], reliable: bool) -> bool;
    fn register(&mut self, username: &str);
    fn host(&mut self, opts: HostOptions);
    fn join(&mut self, room: &str, code: Option<&str>, secret: Option<&str>);
    fn join_code(&mut self, code: &str);
    fn list_rooms(&mut self, has_space: bool);
    fn answer_knock(&mut self, knock: &str, accept: bool);
    fn kick(&mut self, account: i64, ban: bool);
    fn set_closed(&mut self, closed: bool);
    fn set_metadata(&mut self, metadata: BTreeMap<String, String>);
    /// Leave the room (as host: close it).
    fn leave(&mut self);
    fn username(&self) -> Option<String>;
    fn account_id(&self) -> i64;
    fn peer_rtt(&self, peer: PeerId) -> Option<Duration>;
    fn peer_relayed(&self, peer: PeerId) -> Option<bool>;
    fn force_relay(&mut self, on: bool);
    /// Testing: vanish without a goodbye, like a crashed process (the
    /// others notice by time-outs). Real clients: nothing (exit instead).
    fn crash(&mut self) {}
    /// The Warpzone client behind this link, if it is one.
    fn warpzone(&mut self) -> Option<&mut Client> {
        None
    }
}

impl Link for Client {
    fn poll(&mut self) {
        Client::poll(self)
    }
    fn next_event(&mut self) -> Option<Event> {
        Client::next_event(self)
    }
    fn send(&mut self, peer: PeerId, channel: u8, data: &[u8], reliable: bool) -> bool {
        Client::send(self, peer, channel, data, reliable)
    }
    fn register(&mut self, username: &str) {
        Client::register(self, username)
    }
    fn host(&mut self, opts: HostOptions) {
        Client::host(self, opts)
    }
    fn join(&mut self, room: &str, code: Option<&str>, secret: Option<&str>) {
        Client::join(self, room, code, secret)
    }
    fn join_code(&mut self, code: &str) {
        Client::join_code(self, code)
    }
    fn list_rooms(&mut self, has_space: bool) {
        Client::list_rooms(self, has_space)
    }
    fn answer_knock(&mut self, knock: &str, accept: bool) {
        Client::answer_knock(self, knock, accept)
    }
    fn kick(&mut self, account: i64, ban: bool) {
        Client::kick(self, account, ban)
    }
    fn set_closed(&mut self, closed: bool) {
        Client::set_closed(self, closed)
    }
    fn set_metadata(&mut self, metadata: BTreeMap<String, String>) {
        Client::set_metadata(self, metadata)
    }
    fn leave(&mut self) {
        Client::leave(self)
    }
    fn username(&self) -> Option<String> {
        self.account().map(|a| a.username)
    }
    fn account_id(&self) -> i64 {
        self.account().map(|a| a.id).unwrap_or(0)
    }
    fn peer_rtt(&self, peer: PeerId) -> Option<Duration> {
        Client::peer_rtt(self, peer)
    }
    fn peer_relayed(&self, peer: PeerId) -> Option<bool> {
        Client::peer_relayed(self, peer)
    }
    fn force_relay(&mut self, on: bool) {
        Client::force_relay(self, on)
    }
    fn warpzone(&mut self) -> Option<&mut Client> {
        Some(self)
    }
}

/// Artificial network conditions on everything this machine sends. The
/// session channel is reliable and ordered, so a "lost" packet is not lost:
/// it arrives one retransmission timeout later, and everything sent after it
/// to the same peer waits behind it (head-of-line blocking), which is what
/// loss does to a reliable stream.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkSim {
    /// One-way delay added to every chunk.
    pub delay_ms: u32,
    /// Uniform extra delay in `-jitter..=+jitter` (never reorders: the
    /// channel is ordered).
    pub jitter_ms: u32,
    /// Probability (0..1) that a transmission is lost and repeated.
    pub loss: f32,
}

impl LinkSim {
    /// `BABA_NET_DELAY_MS`, `BABA_NET_JITTER_MS`, `BABA_NET_LOSS` (percent).
    pub fn from_env() -> LinkSim {
        let num = |k: &str| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(0.0)
        };
        LinkSim {
            delay_ms: num("BABA_NET_DELAY_MS").max(0.0) as u32,
            jitter_ms: num("BABA_NET_JITTER_MS").max(0.0) as u32,
            loss: (num("BABA_NET_LOSS") / 100.0).clamp(0.0, 0.9),
        }
    }

    pub fn active(&self) -> bool {
        self.delay_ms > 0 || self.jitter_ms > 0 || self.loss > 0.0
    }

    /// Retransmission timeout of the simulated reliable channel.
    fn rto_ms(&self) -> f32 {
        (3 * self.delay_ms).max(60) as f32
    }
}

/// Traffic counters of this machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrafficStats {
    pub sent_msgs: u64,
    pub sent_chunks: u64,
    pub sent_bytes: u64,
    pub recv_msgs: u64,
    pub recv_bytes: u64,
    /// Transmissions the link conditioner "lost" (and repeated).
    pub sim_lost: u64,
    /// Not sendable (peer gone).
    pub unroutable: u64,
}

/// Outgoing queue with the link conditioner.
pub(crate) struct Outbox {
    pub sim: LinkSim,
    queues: BTreeMap<PeerId, VecDeque<(Instant, u8, Vec<u8>)>>,
    last: BTreeMap<PeerId, Instant>,
    rng: u64,
}

impl Outbox {
    pub fn new(sim: LinkSim) -> Outbox {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            ^ ((std::process::id() as u64) << 32);
        Outbox {
            sim,
            queues: BTreeMap::new(),
            last: BTreeMap::new(),
            rng: seed | 1,
        }
    }

    pub fn reseed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }

    fn rand01(&mut self) -> f32 {
        // xorshift64*: link simulation only, never game state.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Queue (or send at once) one chunk.
    pub fn submit(
        &mut self,
        link: &mut dyn Link,
        stats: &mut TrafficStats,
        peer: PeerId,
        channel: u8,
        data: Vec<u8>,
    ) {
        stats.sent_chunks += 1;
        stats.sent_bytes += data.len() as u64;
        if !self.sim.active() {
            if !link.send(peer, channel, &data, true) {
                stats.unroutable += 1;
            }
            return;
        }
        let j = self.sim.jitter_ms as f32;
        let mut ms = (self.sim.delay_ms as f32 + (self.rand01() * 2.0 - 1.0) * j).max(0.0);
        let mut tries = 0;
        while self.sim.loss > 0.0 && tries < 20 && self.rand01() < self.sim.loss {
            ms += self.sim.rto_ms();
            stats.sim_lost += 1;
            tries += 1;
        }
        let now = Instant::now();
        let mut at = now + Duration::from_micros((ms * 1000.0) as u64);
        if let Some(&l) = self.last.get(&peer) {
            at = at.max(l);
        }
        self.last.insert(peer, at);
        self.queues
            .entry(peer)
            .or_default()
            .push_back((at, channel, data));
    }

    /// Send what is due.
    pub fn flush(&mut self, link: &mut dyn Link, stats: &mut TrafficStats) {
        let now = Instant::now();
        for (peer, q) in self.queues.iter_mut() {
            while q.front().is_some_and(|(at, _, _)| *at <= now) {
                let (_, ch, d) = q.pop_front().unwrap();
                if !link.send(*peer, ch, &d, true) {
                    stats.unroutable += 1;
                }
            }
        }
        self.queues.retain(|_, q| !q.is_empty());
    }

    /// Forget a peer (it left).
    pub fn drop_peer(&mut self, peer: PeerId) {
        self.queues.remove(&peer);
        self.last.remove(&peer);
    }

    pub fn clear(&mut self) {
        self.queues.clear();
        self.last.clear();
    }

    /// Chunks still waiting in the conditioner.
    pub fn backlog(&self) -> usize {
        self.queues.values().map(|q| q.len()).sum()
    }
}
