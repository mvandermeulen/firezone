use boringtun::x25519::{PublicKey, StaticSecret};
use firezone_relay::{AddressFamily, AllocationPort, ClientSocket, IpStack, PeerSocket};
use rand::rngs::OsRng;
use snownet::{Answer, ClientNode, Event, MutableIpPacket, ServerNode, Transmit};
use std::{
    collections::HashSet,
    iter,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    time::{Duration, Instant, SystemTime},
    vec,
};
use str0m::{net::Protocol, Candidate};
use tracing::{debug_span, info_span, Span};
use tracing_subscriber::util::SubscriberInitExt;

#[test]
fn smoke_direct() {
    let _guard = setup_tracing();

    let (alice, bob) = alice_and_bob();

    let mut alice =
        TestNode::new(info_span!("Alice"), alice, "1.1.1.1:80").with_primary_as_host_candidate();
    let mut bob =
        TestNode::new(info_span!("Bob"), bob, "1.1.1.2:80").with_primary_as_host_candidate();
    let firewall = Firewall::default();
    let mut clock = Clock::new();

    handshake(&mut alice, &mut bob, &[], &clock);

    loop {
        if alice.is_connected_to(&bob) && bob.is_connected_to(&alice) {
            break;
        }

        progress(&mut alice, &mut bob, &mut [], &firewall, &mut clock);
    }
}

#[test]
fn smoke_relayed() {
    let _guard = setup_tracing();

    let (alice, bob) = alice_and_bob();

    let relay = TestRelay::new(IpAddr::V4(Ipv4Addr::LOCALHOST), debug_span!("Roger"));
    let mut alice = TestNode::new(debug_span!("Alice"), alice, "1.1.1.1:80");
    let mut bob = TestNode::new(debug_span!("Bob"), bob, "2.2.2.2:80");
    let firewall = Firewall::default()
        .with_block_rule("1.1.1.1:80", "2.2.2.2:80")
        .with_block_rule("2.2.2.2:80", "1.1.1.1:80");
    let mut clock = Clock::new();

    let mut relays = [relay];

    handshake(&mut alice, &mut bob, &relays, &clock);

    loop {
        if alice.is_connected_to(&bob) && bob.is_connected_to(&alice) {
            break;
        }

        progress(&mut alice, &mut bob, &mut relays, &firewall, &mut clock);
    }
}

#[test]
fn reconnect_discovers_new_interface() {
    let _guard = setup_tracing();

    let (alice, bob) = alice_and_bob();

    let relay = TestRelay::new(IpAddr::V4(Ipv4Addr::LOCALHOST), debug_span!("Roger"));
    let mut alice = TestNode::new(debug_span!("Alice"), alice, "1.1.1.1:80");
    let mut bob = TestNode::new(debug_span!("Bob"), bob, "2.2.2.2:80");
    let firewall = Firewall::default();
    let mut clock = Clock::new();

    let mut relays = [relay];

    handshake(&mut alice, &mut bob, &relays, &clock);

    loop {
        if alice.is_connected_to(&bob) && bob.is_connected_to(&alice) {
            break;
        }

        progress(&mut alice, &mut bob, &mut relays, &firewall, &mut clock);
    }

    alice.switch_network("10.0.0.1:80");
    alice.node.reconnect(clock.now);

    // Make some progress.
    for _ in 0..10 {
        progress(&mut alice, &mut bob, &mut relays, &firewall, &mut clock);
    }

    assert!(alice
        .signalled_candidates()
        .any(|(_, c, _)| c.addr().to_string() == "10.0.0.1:80"));
    assert_eq!(alice.failed_connections().count(), 0);
    assert_eq!(bob.failed_connections().count(), 0);
}

#[test]
fn connection_times_out_after_20_seconds() {
    let (mut alice, _) = alice_and_bob();

    let created_at = Instant::now();

    let _ = alice.new_connection(
        1,
        HashSet::new(),
        HashSet::new(),
        Instant::now(),
        created_at,
    );
    alice.handle_timeout(created_at + Duration::from_secs(20));

    assert_eq!(alice.poll_event().unwrap(), Event::ConnectionFailed(1));
}

#[test]
fn connection_without_candidates_times_out_after_10_seconds() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let start = Instant::now();

    let (mut alice, mut bob) = alice_and_bob();
    let answer = send_offer(&mut alice, &mut bob, start);

    let accepted_at = start + Duration::from_secs(1);
    alice.accept_answer(1, bob.public_key(), answer, accepted_at);

    alice.handle_timeout(accepted_at + Duration::from_secs(10));

    assert_eq!(alice.poll_event().unwrap(), Event::ConnectionFailed(1));
}

#[test]
fn connection_with_candidates_does_not_time_out_after_10_seconds() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let start = Instant::now();

    let (mut alice, mut bob) = alice_and_bob();
    let answer = send_offer(&mut alice, &mut bob, start);

    let accepted_at = start + Duration::from_secs(1);
    alice.accept_answer(1, bob.public_key(), answer, accepted_at);
    alice.add_local_host_candidate(s("10.0.0.2:4444")).unwrap();
    alice.add_remote_candidate(1, host("10.0.0.1:4444"), accepted_at);

    alice.handle_timeout(accepted_at + Duration::from_secs(10));

    let any_failed =
        iter::from_fn(|| alice.poll_event()).any(|e| matches!(e, Event::ConnectionFailed(_)));

    assert!(!any_failed);
}

#[test]
fn answer_after_stale_connection_does_not_panic() {
    let start = Instant::now();

    let (mut alice, mut bob) = alice_and_bob();
    let answer = send_offer(&mut alice, &mut bob, start);

    let now = start + Duration::from_secs(10);
    alice.handle_timeout(now);

    alice.accept_answer(1, bob.public_key(), answer, now + Duration::from_secs(1));
}

#[test]
fn only_generate_candidate_event_after_answer() {
    let local_candidate = SocketAddr::new(IpAddr::from(Ipv4Addr::LOCALHOST), 10000);

    let mut alice = ClientNode::<u64>::new(StaticSecret::random_from_rng(rand::thread_rng()));

    alice.add_local_host_candidate(local_candidate).unwrap();

    let mut bob = ServerNode::<u64>::new(StaticSecret::random_from_rng(rand::thread_rng()));

    let offer = alice.new_connection(
        1,
        HashSet::new(),
        HashSet::new(),
        Instant::now(),
        Instant::now(),
    );

    assert_eq!(
        alice.poll_event(),
        None,
        "no event to be emitted before accepting the answer"
    );

    let answer = bob.accept_connection(
        1,
        offer,
        alice.public_key(),
        HashSet::new(),
        HashSet::new(),
        Instant::now(),
    );

    alice.accept_answer(1, bob.public_key(), answer, Instant::now());

    assert!(iter::from_fn(|| alice.poll_event()).any(|ev| ev
        == Event::SignalIceCandidate {
            connection: 1,
            candidate: Candidate::host(local_candidate, Protocol::Udp)
                .unwrap()
                .to_sdp_string()
        }));
}

#[test]
fn second_connection_with_same_relay_reuses_allocation() {
    let mut alice = ClientNode::<u64>::new(StaticSecret::random_from_rng(rand::thread_rng()));

    let _ = alice.new_connection(
        1,
        HashSet::new(),
        HashSet::from([relay("user1", "pass1", "realm1")]),
        Instant::now(),
        Instant::now(),
    );

    let transmit = alice.poll_transmit().unwrap();
    assert_eq!(transmit.dst, RELAY);
    assert!(alice.poll_transmit().is_none());

    let _ = alice.new_connection(
        2,
        HashSet::new(),
        HashSet::from([relay("user1", "pass1", "realm1")]),
        Instant::now(),
        Instant::now(),
    );

    assert!(alice.poll_transmit().is_none());
}

fn setup_tracing() -> tracing::subscriber::DefaultGuard {
    tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("debug")
        .finish()
        .set_default()
}

fn alice_and_bob() -> (ClientNode<u64>, ServerNode<u64>) {
    let alice = ClientNode::<u64>::new(StaticSecret::random_from_rng(rand::thread_rng()));
    let bob = ServerNode::<u64>::new(StaticSecret::random_from_rng(rand::thread_rng()));

    (alice, bob)
}

fn send_offer(alice: &mut ClientNode<u64>, bob: &mut ServerNode<u64>, now: Instant) -> Answer {
    let offer = alice.new_connection(1, HashSet::new(), HashSet::new(), Instant::now(), now);

    bob.accept_connection(
        1,
        offer,
        alice.public_key(),
        HashSet::new(),
        HashSet::new(),
        now,
    )
}

fn relay(username: &str, pass: &str, realm: &str) -> (SocketAddr, String, String, String) {
    (
        RELAY,
        username.to_owned(),
        pass.to_owned(),
        realm.to_owned(),
    )
}

fn host(socket: &str) -> String {
    Candidate::host(s(socket), Protocol::Udp)
        .unwrap()
        .to_sdp_string()
}

fn s(socket: &str) -> SocketAddr {
    socket.parse().unwrap()
}

const RELAY: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 10000));

// Heavily inspired by https://github.com/algesten/str0m/blob/7ed5143381cf095f7074689cc254b8c9e50d25c5/src/ice/mod.rs#L547-L647.
struct TestNode {
    node: EitherNode,
    span: Span,
    received_packets: Vec<MutableIpPacket<'static>>,
    /// The primary interface we use to send packets (e.g. to relays).
    primary: SocketAddr,
    /// All local interfaces.
    local: Vec<SocketAddr>,
    events: Vec<(Event<u64>, Instant)>,

    buffer: Box<[u8; 10_000]>,
}

struct TestRelay {
    inner: firezone_relay::Server<OsRng>,
    listen_addr: SocketAddr,
    span: Span,

    allocations: HashSet<(AddressFamily, AllocationPort)>,
    buffer: Vec<u8>,
}

#[derive(Default)]
struct Firewall {
    blocked: Vec<(SocketAddr, SocketAddr)>,
}

struct Clock {
    start: Instant,
    now: Instant,

    tick_rate: Duration,
    max_time: Instant,
}

impl Clock {
    fn new() -> Self {
        let now = Instant::now();
        let tick_rate = Duration::from_millis(100);
        let one_hour = Duration::from_secs(60) * 60;

        Self {
            start: now,
            now,
            tick_rate,
            max_time: now + one_hour,
        }
    }

    fn tick(&mut self) {
        self.now += self.tick_rate;

        let elapsed = self.now.duration_since(self.start);

        if elapsed.as_millis() % 60_000 == 0 {
            tracing::info!("Time since start: {elapsed:?}")
        }

        if self.now >= self.max_time {
            panic!("Time exceeded")
        }
    }
}

impl Firewall {
    fn with_block_rule(mut self, from: &str, to: &str) -> Self {
        self.blocked
            .push((from.parse().unwrap(), to.parse().unwrap()));

        self
    }
}

impl TestRelay {
    fn new(local: IpAddr, span: Span) -> Self {
        let inner = firezone_relay::Server::new(IpStack::from(local), OsRng, 49152, 65535);

        Self {
            inner,
            listen_addr: SocketAddr::from((local, 3478)),
            span,
            allocations: HashSet::default(),
            buffer: vec![0u8; (1 << 16) - 1],
        }
    }

    fn wants(&self, dst: SocketAddr) -> bool {
        self.listen_addr == dst
            || self.allocations.contains(&match dst {
                SocketAddr::V4(_) => (AddressFamily::V4, AllocationPort::new(dst.port())),
                SocketAddr::V6(_) => (AddressFamily::V6, AllocationPort::new(dst.port())),
            })
    }

    fn ip(&self) -> IpAddr {
        self.listen_addr.ip()
    }

    fn handle_packet(
        &mut self,
        payload: &[u8],
        sender: SocketAddr,
        dst: SocketAddr,
        other: &mut TestNode,
        now: Instant,
    ) {
        if dst == self.listen_addr {
            self.handle_client_input(payload, ClientSocket::new(sender), other, now);
            return;
        }

        self.handle_peer_traffic(
            payload,
            PeerSocket::new(sender),
            AllocationPort::new(dst.port()),
            other,
            now,
        )
    }

    fn handle_client_input(
        &mut self,
        payload: &[u8],
        client: ClientSocket,
        receiver: &mut TestNode,
        now: Instant,
    ) {
        if let Some((port, peer)) = self
            .span
            .in_scope(|| self.inner.handle_client_input(payload, client, now))
        {
            let payload = &payload[4..];

            // The `src` of the relayed packet is the relay itself _from_ the allocated port.
            let src = SocketAddr::new(self.ip(), port.value());

            // The `dst` of the relayed packet is what TURN calls a "peer".
            let dst = peer.into_socket();

            // Check if we need to relay to ourselves (from one allocation to another)
            if self.wants(dst) {
                // When relaying to ourselves, we become our own peer.
                let peer_socket = PeerSocket::new(src);
                // The allocation that the data is arriving on is the `dst`'s port.
                let allocation_port = AllocationPort::new(dst.port());

                self.handle_peer_traffic(payload, peer_socket, allocation_port, receiver, now);

                return;
            }

            receiver.receive(dst, src, payload, now);
        }
    }

    fn handle_peer_traffic(
        &mut self,
        payload: &[u8],
        peer: PeerSocket,
        port: AllocationPort,
        receiver: &mut TestNode,
        now: Instant,
    ) {
        if let Some((client, channel)) = self
            .span
            .in_scope(|| self.inner.handle_peer_traffic(payload, peer, port))
        {
            let full_length = firezone_relay::ChannelData::encode_header_to_slice(
                channel,
                payload.len() as u16,
                &mut self.buffer[..4],
            );
            self.buffer[4..full_length].copy_from_slice(payload);

            receiver.receive(
                client.into_socket(),
                self.listen_addr,
                &self.buffer[..full_length],
                now,
            );
        }
    }

    fn drain_messages(&mut self, a1: &mut TestNode, a2: &mut TestNode, now: Instant) {
        while let Some(command) = self.inner.next_command() {
            match command {
                firezone_relay::Command::SendMessage { payload, recipient } => {
                    let recipient = recipient.into_socket();
                    if a1.local.contains(&recipient) {
                        a1.receive(recipient, self.listen_addr, &payload, now);
                        continue;
                    }

                    if a2.local.contains(&recipient) {
                        a2.receive(recipient, self.listen_addr, &payload, now);
                        continue;
                    }

                    panic!("Relay generated traffic for unknown client")
                }
                firezone_relay::Command::CreateAllocation { port, family } => {
                    self.allocations.insert((family, port));
                }
                firezone_relay::Command::FreeAllocation { port, family } => {
                    self.allocations.remove(&(family, port));
                }
            }
        }
    }

    fn make_credentials(&self, username: &str) -> (String, String) {
        let expiry = SystemTime::now() + Duration::from_secs(60);

        let secs = expiry
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("expiry must be later than UNIX_EPOCH")
            .as_secs();

        let password =
            firezone_relay::auth::generate_password(self.inner.auth_secret(), expiry, username);

        (format!("{secs}:{username}"), password)
    }
}

enum EitherNode {
    Server(ServerNode<u64>),
    Client(ClientNode<u64>),
}

impl From<ClientNode<u64>> for EitherNode {
    fn from(value: ClientNode<u64>) -> Self {
        Self::Client(value)
    }
}

impl From<ServerNode<u64>> for EitherNode {
    fn from(value: ServerNode<u64>) -> Self {
        Self::Server(value)
    }
}

impl EitherNode {
    fn poll_transmit(&mut self) -> Option<Transmit> {
        match self {
            EitherNode::Client(n) => n.poll_transmit(),
            EitherNode::Server(n) => n.poll_transmit(),
        }
    }

    fn poll_event(&mut self) -> Option<Event<u64>> {
        match self {
            EitherNode::Client(n) => n.poll_event(),
            EitherNode::Server(n) => n.poll_event(),
        }
    }

    fn poll_timeout(&mut self) -> Option<Instant> {
        match self {
            EitherNode::Client(n) => n.poll_timeout(),
            EitherNode::Server(n) => n.poll_timeout(),
        }
    }

    fn add_remote_candidate(&mut self, id: u64, candidate: String, now: Instant) {
        match self {
            EitherNode::Client(n) => n.add_remote_candidate(id, candidate, now),
            EitherNode::Server(n) => n.add_remote_candidate(id, candidate, now),
        }
    }

    fn add_local_host_candidate(&mut self, socket: SocketAddr) {
        match self {
            EitherNode::Client(n) => n.add_local_host_candidate(socket).unwrap(),
            EitherNode::Server(n) => n.add_local_host_candidate(socket).unwrap(),
        }
    }

    fn is_connected_to(&self, key: PublicKey) -> bool {
        match self {
            EitherNode::Client(n) => n.is_connected_to(key),
            EitherNode::Server(n) => n.is_connected_to(key),
        }
    }

    fn public_key(&self) -> PublicKey {
        match self {
            EitherNode::Client(n) => n.public_key(),
            EitherNode::Server(n) => n.public_key(),
        }
    }

    fn as_client_mut(&mut self) -> Option<&mut ClientNode<u64>> {
        match self {
            EitherNode::Server(_) => None,
            EitherNode::Client(c) => Some(c),
        }
    }

    fn as_server_mut(&mut self) -> Option<&mut ServerNode<u64>> {
        match self {
            EitherNode::Server(s) => Some(s),
            EitherNode::Client(_) => None,
        }
    }

    fn decapsulate<'s>(
        &mut self,
        local: SocketAddr,
        from: SocketAddr,
        packet: &[u8],
        now: Instant,
        buffer: &'s mut [u8],
    ) -> Result<Option<(u64, MutableIpPacket<'s>)>, snownet::Error> {
        match self {
            EitherNode::Client(n) => n.decapsulate(local, from, packet, now, buffer),
            EitherNode::Server(n) => n.decapsulate(local, from, packet, now, buffer),
        }
    }

    fn handle_timeout(&mut self, now: Instant) {
        match self {
            EitherNode::Client(n) => n.handle_timeout(now),
            EitherNode::Server(n) => n.handle_timeout(now),
        }
    }

    fn reconnect(&mut self, now: Instant) {
        match self {
            EitherNode::Client(n) => n.reconnect(now),
            EitherNode::Server(n) => n.reconnect(now),
        }
    }
}

impl TestNode {
    pub fn new(span: Span, node: impl Into<EitherNode>, primary: &str) -> Self {
        let primary = primary.parse().unwrap();

        TestNode {
            node: node.into(),
            span,
            received_packets: vec![],
            buffer: Box::new([0u8; 10_000]),
            primary,
            local: vec![primary],
            events: Default::default(),
        }
    }

    fn switch_network(&mut self, new_primary: &str) {
        self.primary = new_primary.parse().unwrap();
        self.local.push(self.primary);
    }

    fn is_connected_to(&self, other: &TestNode) -> bool {
        self.node.is_connected_to(other.node.public_key())
    }

    fn signalled_candidates(&self) -> impl Iterator<Item = (u64, Candidate, Instant)> + '_ {
        self.events.iter().filter_map(|(e, instant)| match e {
            Event::SignalIceCandidate {
                connection,
                candidate,
            } => Some((
                *connection,
                Candidate::from_sdp_string(candidate).unwrap(),
                *instant,
            )),
            Event::ConnectionEstablished(_) | Event::ConnectionFailed(_) => None,
        })
    }

    fn failed_connections(&self) -> impl Iterator<Item = (u64, Instant)> + '_ {
        self.events.iter().filter_map(|(e, instant)| match e {
            Event::ConnectionFailed(id) => Some((*id, *instant)),
            Event::SignalIceCandidate { .. } => None,
            Event::ConnectionEstablished(_) => None,
        })
    }

    fn receive(&mut self, local: SocketAddr, from: SocketAddr, packet: &[u8], now: Instant) {
        if let Some((_, packet)) = self
            .span
            .in_scope(|| {
                self.node
                    .decapsulate(local, from, packet, now, self.buffer.as_mut())
            })
            .unwrap()
        {
            self.received_packets.push(packet.to_owned())
        }
    }

    fn drain_events(&mut self, other: &mut TestNode, now: Instant) {
        while let Some(v) = self.span.in_scope(|| self.node.poll_event()) {
            self.events.push((v.clone(), now));

            match v {
                Event::SignalIceCandidate {
                    connection,
                    candidate,
                } => other.node.add_remote_candidate(connection, candidate, now),
                Event::ConnectionEstablished(_) => {}
                Event::ConnectionFailed(_) => {}
            };
        }
    }

    fn drain_transmits(
        &mut self,
        other: &mut TestNode,
        relays: &mut [TestRelay],
        firewall: &Firewall,
        now: Instant,
    ) {
        while let Some(trans) = self.span.in_scope(|| self.node.poll_transmit()) {
            let payload = &trans.payload;
            let dst = trans.dst;

            if let Some(relay) = relays.iter_mut().find(|r| r.wants(trans.dst)) {
                relay.handle_packet(payload, self.primary, dst, other, now);
                continue;
            }

            let src = trans
                .src
                .expect("transmits without `src` should always be handled by relays");

            // Wasn't traffic for the relay, let's check our firewall.
            if firewall.blocked.contains(&(src, dst)) {
                tracing::debug!(target: "firewall", %src, %dst, "Dropping packet");
                continue;
            }

            // Firewall allowed traffic, let's dispatch it.
            other.receive(dst, src, payload, now);
        }
    }

    fn with_primary_as_host_candidate(mut self) -> Self {
        self.span
            .in_scope(|| self.node.add_local_host_candidate(self.primary));

        self
    }
}

fn handshake(client: &mut TestNode, server: &mut TestNode, relays: &[TestRelay], clock: &Clock) {
    let client_node = &mut client.node.as_client_mut().unwrap();
    let server_node = &mut server.node.as_server_mut().unwrap();

    let client_credentials = relays.iter().map(|relay| {
        let (username, password) = relay.make_credentials("client");

        (relay.listen_addr, username, password, "firezone".to_owned())
    });
    let server_credentials = relays.iter().map(|relay| {
        let (username, password) = relay.make_credentials("client");

        (relay.listen_addr, username, password, "firezone".to_owned())
    });

    let offer = client.span.in_scope(|| {
        client_node.new_connection(
            1,
            HashSet::default(),
            HashSet::from_iter(client_credentials),
            clock.now,
            clock.now,
        )
    });
    let answer = server.span.in_scope(|| {
        server_node.accept_connection(
            1,
            offer,
            client_node.public_key(),
            HashSet::default(),
            HashSet::from_iter(server_credentials),
            clock.now,
        )
    });
    client
        .span
        .in_scope(|| client_node.accept_answer(1, server_node.public_key(), answer, clock.now));
}

fn progress(
    a1: &mut TestNode,
    a2: &mut TestNode,
    relays: &mut [TestRelay],
    firewall: &Firewall,
    clock: &mut Clock,
) {
    clock.tick();

    a1.drain_events(a2, clock.now);
    a2.drain_events(a1, clock.now);

    a1.drain_transmits(a2, relays, firewall, clock.now);
    a2.drain_transmits(a1, relays, firewall, clock.now);

    for relay in relays.iter_mut() {
        relay.drain_messages(a1, a2, clock.now);
    }

    if let Some(timeout) = a1.node.poll_timeout() {
        if clock.now >= timeout {
            a1.span.in_scope(|| a1.node.handle_timeout(clock.now));
        }
    }

    if let Some(timeout) = a2.node.poll_timeout() {
        if clock.now >= timeout {
            a2.span.in_scope(|| a2.node.handle_timeout(clock.now));
        }
    }

    for relay in relays {
        if let Some(timeout) = relay.inner.poll_timeout() {
            if clock.now >= timeout {
                relay
                    .span
                    .in_scope(|| relay.inner.handle_timeout(clock.now))
            }
        }
    }
}
