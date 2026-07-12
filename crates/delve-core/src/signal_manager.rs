//! Centralized signal evaluation for the M2 signal system.
//!
//! Sources (levers, plates, triggers, tripwires) emit boolean signals.
//! Receivers (doors, launchers, ...) compute their active state from incoming
//! signals. Gates are both receivers and sources — they transform signals.
//!
//! Where the TS manager notifies via stored callbacks, this port returns the
//! events produced by each call; the caller applies them to world state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalMode {
    Toggle,
    Momentary,
    OneShot,
    Timed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateMode {
    Or,
    And,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateType {
    And,
    Or,
    Not,
    Delay,
    PulseEdge,
    PulseRepeat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalSource {
    pub entity_id: String,
    /// Receiver entity IDs.
    pub targets: Vec<String>,
    pub signal_mode: SignalMode,
    pub active: bool,
    /// For one_shot: already triggered.
    pub fired: bool,
    /// For timed: total duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// For timed: absolute time to deactivate (0 = not scheduled).
    pub deactivate_at: f64,
    /// Optional activation delay in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    /// Absolute time for delayed activation (0 = not scheduled).
    pub delay_fire_at: f64,
    /// True while waiting for the delay to elapse.
    pub delay_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalReceiver {
    pub entity_id: String,
    pub gate_mode: GateMode,
    /// Computed state.
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalGate {
    pub entity_id: String,
    pub gate_type: GateType,
    /// Output receiver entity IDs.
    pub targets: Vec<String>,
    /// Computed output.
    pub active: bool,
    /// For the delay gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    /// For the pulse_repeat gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<f64>,
    /// Absolute time for the next gate event (0 = not scheduled).
    pub fire_at: f64,
    /// For delay: waiting to activate.
    pub pending_activation: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalEvent {
    ReceiverChanged { entity_id: String, active: bool },
    SourceDeactivated { entity_id: String },
}

/// Full signal state for level snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalManagerState {
    pub sources: Vec<SignalSource>,
    pub receivers: Vec<SignalReceiver>,
    pub gates: Vec<SignalGate>,
    pub now: f64,
}

#[derive(Debug, Default)]
pub struct SignalManager {
    // Registration order is preserved: it drives gate evaluation order and
    // receiver evaluation order exactly like the TS Map iteration order.
    sources: Vec<SignalSource>,
    receivers: Vec<SignalReceiver>,
    gates: Vec<SignalGate>,
    pub now: f64,
}

impl SignalManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_source(
        &mut self,
        entity_id: &str,
        targets: Vec<String>,
        signal_mode: SignalMode,
        duration: Option<f64>,
        delay: Option<f64>,
    ) {
        let source = SignalSource {
            entity_id: entity_id.to_string(),
            targets,
            signal_mode,
            active: false,
            fired: false,
            duration,
            deactivate_at: 0.0,
            delay,
            delay_fire_at: 0.0,
            delay_pending: false,
        };
        if let Some(existing) = self.sources.iter_mut().find(|s| s.entity_id == entity_id) {
            *existing = source;
        } else {
            self.sources.push(source);
        }
    }

    pub fn register_receiver(&mut self, entity_id: &str, gate_mode: GateMode) {
        let receiver = SignalReceiver {
            entity_id: entity_id.to_string(),
            gate_mode,
            active: false,
        };
        if let Some(existing) = self.receivers.iter_mut().find(|r| r.entity_id == entity_id) {
            *existing = receiver;
        } else {
            self.receivers.push(receiver);
        }
    }

    pub fn register_gate(
        &mut self,
        entity_id: &str,
        gate_type: GateType,
        targets: Vec<String>,
        delay: Option<f64>,
        interval: Option<f64>,
    ) {
        let gate = SignalGate {
            entity_id: entity_id.to_string(),
            gate_type,
            targets,
            active: false,
            delay,
            interval,
            fire_at: 0.0,
            pending_activation: false,
        };
        if let Some(existing) = self.gates.iter_mut().find(|g| g.entity_id == entity_id) {
            *existing = gate;
        } else {
            self.gates.push(gate);
        }
    }

    pub fn set_source_active(&mut self, entity_id: &str, active: bool) -> Vec<SignalEvent> {
        let now = self.now;
        let Some(source) = self.sources.iter_mut().find(|s| s.entity_id == entity_id) else {
            return Vec::new();
        };

        // one_shot: ignore re-activation after the first fire.
        if source.signal_mode == SignalMode::OneShot && source.fired {
            return Vec::new();
        }

        // Delayed activation: start the countdown instead of activating now.
        if active
            && source.delay.is_some_and(|delay| delay > 0.0)
            && !source.delay_pending
            && !source.active
        {
            source.delay_pending = true;
            source.delay_fire_at = now + source.delay.expect("checked is_some");
            if source.signal_mode == SignalMode::OneShot {
                source.fired = true;
            }
            return Vec::new(); // tick() propagates once the delay elapses
        }

        // Cancel a pending delay on deactivation.
        if !active {
            source.delay_pending = false;
            source.delay_fire_at = 0.0;
        }

        source.active = active;

        if active && source.signal_mode == SignalMode::OneShot && !source.fired {
            source.fired = true;
        }

        if active
            && source.signal_mode == SignalMode::Timed
            && let Some(duration) = source.duration
        {
            source.deactivate_at = now + duration;
        }

        self.propagate()
    }

    pub fn deactivate_source(&mut self, entity_id: &str) -> Vec<SignalEvent> {
        let Some(source) = self.sources.iter_mut().find(|s| s.entity_id == entity_id) else {
            return Vec::new();
        };
        source.active = false;
        source.delay_pending = false;
        source.delay_fire_at = 0.0;
        source.deactivate_at = 0.0;
        self.propagate()
    }

    /// Advance timed sources and delay/pulse gates.
    pub fn tick(&mut self, delta: f64) -> Vec<SignalEvent> {
        self.now += delta;
        let now = self.now;
        let mut changed = false;
        let mut events = Vec::new();

        // Delayed source activation.
        for source in &mut self.sources {
            if source.delay_pending && source.delay_fire_at > 0.0 && now >= source.delay_fire_at {
                source.delay_pending = false;
                source.active = true;
                if source.signal_mode == SignalMode::Timed
                    && let Some(duration) = source.duration
                {
                    // Schedule deactivation from the INTENDED activation time.
                    source.deactivate_at = source.delay_fire_at + duration;
                }
                source.delay_fire_at = 0.0;
                changed = true;
            }
        }

        // Timed sources: deactivate at the scheduled time.
        for source in &mut self.sources {
            if source.signal_mode == SignalMode::Timed
                && source.active
                && source.deactivate_at > 0.0
                && now >= source.deactivate_at
            {
                source.deactivate_at = 0.0;
                source.active = false;
                changed = true;
                events.push(SignalEvent::SourceDeactivated {
                    entity_id: source.entity_id.clone(),
                });
            }
        }

        // Delay gates and pulse-repeat gates.
        for gate in &mut self.gates {
            if gate.gate_type == GateType::Delay
                && gate.pending_activation
                && gate.fire_at > 0.0
                && now >= gate.fire_at
            {
                gate.fire_at = 0.0;
                gate.pending_activation = false;
                gate.active = true;
                changed = true;
            }
            if gate.gate_type == GateType::PulseRepeat
                && gate.active
                && gate.fire_at > 0.0
                && now >= gate.fire_at
            {
                // NOT now + interval — drift-free.
                gate.fire_at += gate.interval.unwrap_or(1.0);
                changed = true;
            }
        }

        if changed {
            events.extend(self.propagate());
        }
        events
    }

    #[must_use]
    pub fn is_receiver_active(&self, entity_id: &str) -> bool {
        self.get_receiver(entity_id).is_some_and(|r| r.active)
    }

    #[must_use]
    pub fn is_source_active(&self, entity_id: &str) -> bool {
        self.get_source(entity_id).is_some_and(|s| s.active)
    }

    #[must_use]
    pub fn get_source(&self, entity_id: &str) -> Option<&SignalSource> {
        self.sources.iter().find(|s| s.entity_id == entity_id)
    }

    #[must_use]
    pub fn get_receiver(&self, entity_id: &str) -> Option<&SignalReceiver> {
        self.receivers.iter().find(|r| r.entity_id == entity_id)
    }

    #[must_use]
    pub fn get_gate(&self, entity_id: &str) -> Option<&SignalGate> {
        self.gates.iter().find(|g| g.entity_id == entity_id)
    }

    /// Clear all registrations.
    pub fn clear(&mut self) {
        self.sources.clear();
        self.receivers.clear();
        self.gates.clear();
        self.now = 0.0;
    }

    /// Save signal state for a level snapshot.
    #[must_use]
    pub fn save_state(&self) -> SignalManagerState {
        SignalManagerState {
            sources: self.sources.clone(),
            receivers: self.receivers.clone(),
            gates: self.gates.clone(),
            now: self.now,
        }
    }

    /// Restore signal state from a level snapshot.
    pub fn load_state(&mut self, state: SignalManagerState) {
        self.now = state.now;
        self.sources = state.sources;
        self.receivers = state.receivers;
        self.gates = state.gates;
    }

    /// Propagate signal state from sources through gates to receivers.
    /// Returns receiver-changed events.
    pub fn propagate(&mut self) -> Vec<SignalEvent> {
        // 1. Topologically sort gates so upstream gates evaluate before
        //    downstream. Gates in cycles are skipped (cycle guard).
        let sorted_gate_indices = self.topological_sort_gates();

        // 2. Evaluate gates in order, collecting inputs from sources + other gates.
        for gate_index in sorted_gate_indices {
            let gate_id = self.gates[gate_index].entity_id.clone();
            let mut inputs: Vec<bool> = Vec::new();
            for source in &self.sources {
                if source.targets.contains(&gate_id) {
                    inputs.push(source.active);
                }
            }
            for (other_index, other) in self.gates.iter().enumerate() {
                if other_index == gate_index {
                    continue;
                }
                if other.targets.contains(&gate_id) {
                    inputs.push(other.active);
                }
            }

            let input_active = inputs.iter().any(|&v| v);
            let now = self.now;
            let gate = &mut self.gates[gate_index];

            match gate.gate_type {
                GateType::And => gate.active = evaluate_gate_mode(GateMode::And, &inputs),
                GateType::Or => gate.active = evaluate_gate_mode(GateMode::Or, &inputs),
                GateType::Not => gate.active = !input_active,
                GateType::Delay => {
                    if input_active && !gate.pending_activation && !gate.active {
                        gate.pending_activation = true;
                        gate.fire_at = now + gate.delay.unwrap_or(0.0);
                    }
                    if !input_active {
                        gate.pending_activation = false;
                        gate.active = false;
                    }
                }
                GateType::PulseEdge => gate.active = input_active && !gate.active,
                GateType::PulseRepeat => {
                    if input_active && !gate.active {
                        gate.active = true;
                        gate.fire_at = now + gate.interval.unwrap_or(1.0);
                    }
                    if !input_active {
                        gate.active = false;
                    }
                }
            }
        }

        // 3. Evaluate receivers from source + gate inputs.
        let mut events = Vec::new();
        for receiver_index in 0..self.receivers.len() {
            let receiver_id = self.receivers[receiver_index].entity_id.clone();
            let mut inputs: Vec<bool> = Vec::new();
            for source in &self.sources {
                if source.targets.contains(&receiver_id) {
                    inputs.push(source.active);
                }
            }
            for gate in &self.gates {
                if gate.targets.contains(&receiver_id) {
                    inputs.push(gate.active);
                }
            }

            let receiver = &mut self.receivers[receiver_index];
            let old_active = receiver.active;
            receiver.active = evaluate_gate_mode(receiver.gate_mode, &inputs);
            if receiver.active != old_active {
                events.push(SignalEvent::ReceiverChanged {
                    entity_id: receiver.entity_id.clone(),
                    active: receiver.active,
                });
            }
        }
        events
    }

    /// Kahn's algorithm over gate→gate edges; gates in cycles are excluded.
    fn topological_sort_gates(&self) -> Vec<usize> {
        let index_of = |id: &str| self.gates.iter().position(|g| g.entity_id == id);

        let mut in_degree = vec![0usize; self.gates.len()];
        for gate in &self.gates {
            for target in &gate.targets {
                if let Some(target_index) = index_of(target) {
                    in_degree[target_index] += 1;
                }
            }
        }

        let mut queue: std::collections::VecDeque<usize> = (0..self.gates.len())
            .filter(|&index| in_degree[index] == 0)
            .collect();

        let mut sorted = Vec::new();
        while let Some(index) = queue.pop_front() {
            sorted.push(index);
            let targets = self.gates[index].targets.clone();
            for target in &targets {
                if let Some(target_index) = index_of(target) {
                    in_degree[target_index] -= 1;
                    if in_degree[target_index] == 0 {
                        queue.push_back(target_index);
                    }
                }
            }
        }
        sorted
    }
}

fn evaluate_gate_mode(mode: GateMode, inputs: &[bool]) -> bool {
    if inputs.is_empty() {
        return false;
    }
    match mode {
        GateMode::Or => inputs.iter().any(|&v| v),
        GateMode::And => inputs.iter().all(|&v| v),
        GateMode::Xor => inputs.iter().filter(|&&v| v).count() % 2 == 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::Mulberry32;

    fn manager() -> SignalManager {
        SignalManager::new()
    }

    fn source(manager: &mut SignalManager, id: &str, targets: &[&str]) {
        manager.register_source(
            id,
            targets.iter().map(ToString::to_string).collect(),
            SignalMode::Toggle,
            None,
            None,
        );
    }

    fn receiver_changed(events: &[SignalEvent], id: &str, active: bool) -> bool {
        events.contains(&SignalEvent::ReceiverChanged {
            entity_id: id.to_string(),
            active,
        })
    }

    #[test]
    fn registers_sources_and_receivers() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);
        assert!(sm.get_source("lever_1").is_some());
        assert!(sm.get_receiver("door_1").is_some());
    }

    #[test]
    fn activating_a_source_activates_its_receiver() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);
        sm.set_source_active("lever_1", true);
        assert!(sm.is_receiver_active("door_1"));
    }

    #[test]
    fn deactivating_a_source_deactivates_its_receiver() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);
        sm.set_source_active("lever_1", true);
        sm.set_source_active("lever_1", false);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn emits_receiver_changed_events_on_state_change() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);

        let on_events = sm.set_source_active("lever_1", true);
        assert!(receiver_changed(&on_events, "door_1", true));

        let off_events = sm.set_source_active("lever_1", false);
        assert!(receiver_changed(&off_events, "door_1", false));
    }

    #[test]
    fn no_events_when_state_unchanged() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);
        let repeat_events = sm.set_source_active("lever_1", true);
        assert!(repeat_events.is_empty());
    }

    #[test]
    fn one_source_activates_multiple_receivers() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1", "door_2"]);
        sm.register_receiver("door_1", GateMode::Or);
        sm.register_receiver("door_2", GateMode::Or);
        sm.set_source_active("lever_1", true);
        assert!(sm.is_receiver_active("door_1"));
        assert!(sm.is_receiver_active("door_2"));
    }

    #[test]
    fn or_mode_any_active_source_activates_receiver() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        source(&mut sm, "lever_2", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);
        assert!(sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_1", false);
        assert!(!sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_2", true);
        assert!(sm.is_receiver_active("door_1"));
    }

    #[test]
    fn and_mode_all_sources_must_be_active() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        source(&mut sm, "lever_2", &["door_1"]);
        sm.register_receiver("door_1", GateMode::And);

        sm.set_source_active("lever_1", true);
        assert!(!sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_2", true);
        assert!(sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_1", false);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn xor_mode_odd_active_sources_activate() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        source(&mut sm, "lever_2", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Xor);

        sm.set_source_active("lever_1", true);
        assert!(sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_2", true);
        assert!(!sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_1", false);
        assert!(sm.is_receiver_active("door_1"));
    }

    #[test]
    fn one_shot_source_can_only_activate_once() {
        let mut sm = manager();
        sm.register_source(
            "trigger_1",
            vec!["door_1".to_string()],
            SignalMode::OneShot,
            None,
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("trigger_1", true);
        assert!(sm.is_receiver_active("door_1"));

        sm.deactivate_source("trigger_1");
        assert!(!sm.is_receiver_active("door_1"));

        sm.set_source_active("trigger_1", true);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn timed_source_auto_deactivates_after_duration() {
        let mut sm = manager();
        sm.register_source(
            "plate_1",
            vec!["door_1".to_string()],
            SignalMode::Timed,
            Some(2.0),
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("plate_1", true);
        assert!(sm.is_receiver_active("door_1"));
        sm.tick(1.0);
        assert!(sm.is_receiver_active("door_1"));
        sm.tick(1.5);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn momentary_source_deactivates_via_deactivate_source() {
        let mut sm = manager();
        sm.register_source(
            "plate_1",
            vec!["door_1".to_string()],
            SignalMode::Momentary,
            None,
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("plate_1", true);
        assert!(sm.is_receiver_active("door_1"));
        sm.deactivate_source("plate_1");
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn and_gate_combines_inputs() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        source(&mut sm, "lever_2", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::And,
            vec!["door_1".to_string()],
            None,
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);
        assert!(!sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_2", true);
        assert!(sm.is_receiver_active("door_1"));
    }

    #[test]
    fn not_gate_inverts_input() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::Not,
            vec!["door_1".to_string()],
            None,
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", false);
        assert!(sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_1", true);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn delay_gate_activates_output_after_delay() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::Delay,
            vec!["door_1".to_string()],
            Some(1.0),
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);
        assert!(!sm.is_receiver_active("door_1"));
        sm.tick(0.5);
        assert!(!sm.is_receiver_active("door_1"));
        sm.tick(0.6);
        assert!(sm.is_receiver_active("door_1"));
    }

    #[test]
    fn save_and_load_preserves_signal_state() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);
        sm.set_source_active("lever_1", true);

        let state = sm.save_state();
        let mut restored = manager();
        restored.load_state(state);
        assert!(restored.is_source_active("lever_1"));
        assert!(restored.is_receiver_active("door_1"));
    }

    #[test]
    fn delayed_source_activates_after_delay_elapses() {
        let mut sm = manager();
        sm.register_source(
            "plate_1",
            vec!["door_1".to_string()],
            SignalMode::Toggle,
            None,
            Some(1.5),
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("plate_1", true);
        assert!(!sm.is_receiver_active("door_1"));
        assert!(!sm.is_source_active("plate_1"));

        sm.tick(1.0);
        assert!(!sm.is_receiver_active("door_1"));
        sm.tick(0.6);
        assert!(sm.is_receiver_active("door_1"));
        assert!(sm.is_source_active("plate_1"));
    }

    #[test]
    fn delayed_timed_source_starts_duration_after_delay() {
        let mut sm = manager();
        sm.register_source(
            "trigger_1",
            vec!["door_1".to_string()],
            SignalMode::Timed,
            Some(1.0),
            Some(0.5),
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("trigger_1", true);
        assert!(!sm.is_receiver_active("door_1"));
        sm.tick(0.6);
        assert!(sm.is_receiver_active("door_1"));
        sm.tick(1.1);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn deactivation_cancels_pending_delay() {
        let mut sm = manager();
        sm.register_source(
            "plate_1",
            vec!["door_1".to_string()],
            SignalMode::Momentary,
            None,
            Some(1.0),
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("plate_1", true);
        assert!(!sm.is_receiver_active("door_1"));
        sm.deactivate_source("plate_1");
        sm.tick(1.5);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn clear_removes_all_registrations() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["door_1"]);
        sm.register_receiver("door_1", GateMode::Or);
        sm.clear();
        assert!(sm.get_source("lever_1").is_none());
        assert!(sm.get_receiver("door_1").is_none());
    }

    #[test]
    fn unknown_entities_are_no_ops() {
        let mut sm = manager();
        sm.set_source_active("nonexistent", true);
        sm.deactivate_source("nonexistent");
        assert!(!sm.is_receiver_active("nonexistent"));
    }

    #[test]
    fn or_gate_chains_through_another_or_gate() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::Or,
            vec!["gate_2".to_string()],
            None,
            None,
        );
        sm.register_gate(
            "gate_2",
            GateType::Or,
            vec!["door_1".to_string()],
            None,
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);
        assert!(sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_1", false);
        assert!(!sm.is_receiver_active("door_1"));
    }

    #[test]
    fn delay_gate_chains_into_another_delay_gate() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::Delay,
            vec!["gate_2".to_string()],
            Some(1.0),
            None,
        );
        sm.register_gate(
            "gate_2",
            GateType::Delay,
            vec!["door_1".to_string()],
            Some(1.0),
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);
        assert!(!sm.is_receiver_active("door_1"));

        sm.tick(1.1);
        assert!(sm.get_gate("gate_1").expect("gate_1 registered").active);
        assert!(!sm.is_receiver_active("door_1"));

        sm.tick(1.1);
        assert!(sm.is_receiver_active("door_1"));
    }

    #[test]
    fn gate_targets_both_another_gate_and_a_receiver() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::Or,
            vec!["door_1".to_string(), "gate_2".to_string()],
            None,
            None,
        );
        sm.register_gate(
            "gate_2",
            GateType::Or,
            vec!["door_2".to_string()],
            None,
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);
        sm.register_receiver("door_2", GateMode::Or);

        sm.set_source_active("lever_1", true);
        assert!(sm.is_receiver_active("door_1"));
        assert!(sm.is_receiver_active("door_2"));
    }

    #[test]
    fn three_deep_gate_chain_propagates() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::Or,
            vec!["gate_2".to_string()],
            None,
            None,
        );
        sm.register_gate(
            "gate_2",
            GateType::Not,
            vec!["gate_3".to_string()],
            None,
            None,
        );
        sm.register_gate(
            "gate_3",
            GateType::Or,
            vec!["door_1".to_string()],
            None,
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);
        assert!(!sm.is_receiver_active("door_1"));
        sm.set_source_active("lever_1", false);
        assert!(sm.is_receiver_active("door_1"));
    }

    #[test]
    fn pulse_repeat_has_zero_drift_over_irregular_deltas() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::PulseRepeat,
            vec!["door_1".to_string()],
            None,
            Some(1.0),
        );
        sm.register_receiver("door_1", GateMode::Or);
        sm.set_source_active("lever_1", true);

        let deltas = [
            0.016, 0.032, 0.008, 0.1, 0.05, 0.004, 0.016, 0.033, 0.016, 0.016, 0.08, 0.012, 0.016,
            0.064, 0.016, 0.016, 0.016, 0.1, 0.016, 0.016,
        ];
        let mut rng = Mulberry32::new(7);
        let mut elapsed = 0.0_f64;
        while elapsed < 10.0 {
            let delta = deltas[(rng.next_f64() * deltas.len() as f64).floor() as usize];
            sm.tick(delta);
            elapsed += delta;
        }
        // With interval 1.0 the next fire time stays on the integer grid — no
        // accumulated drift regardless of frame jitter.
        let gate = sm.get_gate("gate_1").expect("gate registered");
        let expected_next_fire = elapsed.ceil();
        assert!(
            (gate.fire_at - expected_next_fire).abs() < 0.5,
            "fire_at {} drifted from expected {expected_next_fire}",
            gate.fire_at
        );
    }

    #[test]
    fn save_load_preserves_clock_and_timed_deactivation() {
        let mut sm = manager();
        sm.register_source(
            "plate_1",
            vec!["door_1".to_string()],
            SignalMode::Timed,
            Some(3.0),
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("plate_1", true);
        sm.tick(2.0);
        assert!(sm.is_receiver_active("door_1"));

        let state = sm.save_state();
        assert_eq!(state.now, 2.0);

        let mut restored = manager();
        restored.load_state(state);
        assert_eq!(restored.now, 2.0);
        assert!(restored.is_receiver_active("door_1"));

        let events = restored.tick(3.0);
        assert!(!restored.is_receiver_active("door_1"));
        assert!(events.contains(&SignalEvent::SourceDeactivated {
            entity_id: "plate_1".to_string()
        }));
    }

    #[test]
    fn delay_gate_chain_exact_timing_under_frame_jitter() {
        let mut sm = manager();
        source(&mut sm, "lever_1", &["gate_1"]);
        sm.register_gate(
            "gate_1",
            GateType::Delay,
            vec!["gate_2".to_string()],
            Some(1.0),
            None,
        );
        sm.register_gate(
            "gate_2",
            GateType::Delay,
            vec!["door_1".to_string()],
            Some(1.0),
            None,
        );
        sm.register_receiver("door_1", GateMode::Or);

        sm.set_source_active("lever_1", true);

        let steps = [
            0.3, 0.15, 0.05, 0.2, 0.1, 0.25, 0.05, 0.15, 0.3, 0.1, 0.15, 0.1,
        ];
        let mut total = 0.0_f64;
        for delta in steps {
            sm.tick(delta);
            total += delta;
            if total < 1.0 {
                assert!(!sm.get_gate("gate_1").expect("gate_1 registered").active);
            }
            if total < 2.0 {
                assert!(!sm.is_receiver_active("door_1"));
            }
        }
        assert!((total - 1.9).abs() < 1e-9);
        assert!(!sm.is_receiver_active("door_1"));

        sm.tick(0.2);
        assert!(sm.is_receiver_active("door_1"));
    }
}
