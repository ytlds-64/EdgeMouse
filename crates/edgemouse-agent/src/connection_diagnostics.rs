//! Timing metadata only: never input events, clipboard contents, or credentials.
//! Observations do not change heartbeat deadlines or input recovery behavior.
use std::time::Instant;

fn age_ms(now: Instant, then: Option<Instant>) -> String {
    then.map(|then| now.saturating_duration_since(then).as_millis().to_string())
        .unwrap_or_else(|| "none".to_owned())
}

#[derive(Default)]
pub(crate) struct LoopProgress {
    last_poll: Option<Instant>,
    max_gap_ms: u128,
    heartbeats: u64,
    last_heartbeat: Option<Instant>,
}

impl LoopProgress {
    pub(crate) fn poll(&mut self, now: Instant) {
        if let Some(previous) = self.last_poll {
            self.max_gap_ms = self
                .max_gap_ms
                .max(now.saturating_duration_since(previous).as_millis());
        }
        self.last_poll = Some(now);
    }

    pub(crate) fn heartbeat(&mut self, now: Instant) {
        self.heartbeats = self.heartbeats.saturating_add(1);
        self.last_heartbeat = Some(now);
    }

    pub(crate) fn summary(&self, label: &str, now: Instant) -> String {
        format!(
            "{label}_poll_age_ms={} {label}_max_gap_ms={} {label}_heartbeats={} {label}_heartbeat_age_ms={}",
            age_ms(now, self.last_poll),
            self.max_gap_ms,
            self.heartbeats,
            age_ms(now, self.last_heartbeat),
        )
    }
}

#[derive(Default)]
pub(crate) struct NetworkProgress {
    pub(crate) receive: LoopProgress,
    queued_heartbeats: u64,
    last_queued_heartbeat: Option<Instant>,
    send_started: Option<(&'static str, Instant)>,
    last_message: Option<Instant>,
    last_movement: Option<Instant>,
    heartbeat_max_gap_ms: u128,
    peer_monotonic_ms: Option<u64>,
    pub(crate) transport_at_heartbeat: Option<(u64, u64)>,
}

impl NetworkProgress {
    pub(crate) fn begin_send(&mut self, kind: &'static str, now: Instant) {
        self.send_started = Some((kind, now));
    }

    pub(crate) fn finish_send(&mut self, now: Instant) {
        if matches!(self.send_started, Some(("heartbeat", _))) {
            self.queued_heartbeats = self.queued_heartbeats.saturating_add(1);
            self.last_queued_heartbeat = Some(now);
        }
        self.send_started = None;
    }

    pub(crate) fn message(&mut self, heartbeat_ms: Option<u64>, now: Instant) {
        self.last_message = Some(now);
        if let Some(monotonic_ms) = heartbeat_ms {
            self.heartbeat(monotonic_ms, now);
        }
    }

    pub(crate) fn heartbeat(&mut self, monotonic_ms: u64, now: Instant) {
        if let Some(previous) = self.receive.last_heartbeat {
            self.heartbeat_max_gap_ms = self
                .heartbeat_max_gap_ms
                .max(now.saturating_duration_since(previous).as_millis());
        }
        self.peer_monotonic_ms = Some(monotonic_ms);
        self.receive.heartbeat(now);
    }

    pub(crate) fn movement(&mut self, now: Instant) {
        self.last_movement = Some(now);
    }

    pub(crate) fn summary(&self, now: Instant) -> String {
        format!(
            "{} heartbeat_queued={} heartbeat_queued_age_ms={} heartbeat_max_gap_ms={} peer_monotonic_ms={:?} reliable_rx_age_ms={} movement_rx_age_ms={} pending_send={} pending_send_age_ms={} udp_rx_at_last_heartbeat={:?} ack_rx_at_last_heartbeat={:?}",
            self.receive.summary("network", now),
            self.queued_heartbeats,
            age_ms(now, self.last_queued_heartbeat),
            self.heartbeat_max_gap_ms,
            self.peer_monotonic_ms,
            age_ms(now, self.last_message),
            age_ms(now, self.last_movement),
            self.send_started.map(|(kind, _)| kind).unwrap_or("none"),
            age_ms(now, self.send_started.map(|(_, started)| started)),
            self.transport_at_heartbeat.map(|(udp, _)| udp),
            self.transport_at_heartbeat.map(|(_, ack)| ack),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn datagram_heartbeat_does_not_hide_a_stalled_reliable_stream() {
        let start = Instant::now();
        let mut network = NetworkProgress::default();
        network.message(None, start);
        let now = start + Duration::from_millis(1600);
        network.heartbeat(1750, now);
        let summary = network.summary(now);
        assert!(summary.contains("network_heartbeat_age_ms=0"));
        assert!(summary.contains("reliable_rx_age_ms=1600"));
    }

    #[test]
    fn fresh_movements_do_not_hide_a_stale_heartbeat_or_a_blocked_write() {
        let start = Instant::now();
        let mut network = NetworkProgress::default();
        network.message(Some(500), start);
        network.begin_send("heartbeat", start);
        let now = start + Duration::from_millis(1600);
        network.movement(now);
        let summary = network.summary(now);
        assert!(summary.contains("network_heartbeat_age_ms=1600"));
        assert!(summary.contains("movement_rx_age_ms=0"));
        assert!(summary.contains("pending_send=heartbeat pending_send_age_ms=1600"));
        assert!(summary.contains("heartbeat_queued=0"));
        network.finish_send(now);
        assert!(network.summary(now).contains("heartbeat_queued=1"));
    }

    #[test]
    fn distinguish_network_reception_from_stalled_input_processing() {
        let start = Instant::now();
        let mut network = NetworkProgress::default();
        let mut input = LoopProgress::default();
        input.poll(start);
        input.heartbeat(start);
        let now = start + Duration::from_millis(1600);
        network.message(Some(2000), now);
        assert!(network.summary(now).contains("network_heartbeat_age_ms=0"));
        assert!(
            input
                .summary("input", now)
                .contains("input_heartbeat_age_ms=1600")
        );
        input.poll(now);
        assert!(
            input
                .summary("input", now)
                .contains("input_max_gap_ms=1600")
        );
    }
}
