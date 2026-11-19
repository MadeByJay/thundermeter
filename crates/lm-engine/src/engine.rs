//! The assembled engine: one thread owning the PipeWire main loop, driven by
//! [`EngineCommand`]s, emitting [`EngineEvent`]s.
//!
//! Everything the spikes proved lives here: in-process filter-chain modules
//! (strips/buses), the link reconciler (routing matrix + meter taps), Props
//! pods for volume/mute/DSP params, and the 30 Hz meter drain.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use lm_protocol::{
    AppState, BusId, BusState, CompParams, DeviceInfo, EngineCommand, EngineEvent, EqParams,
    GateParams, LimiterParams, StripId, StripKind, StripState, Target,
};
use pipewire::context::ContextRc;
use pipewire::core::CoreRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::metadata::Metadata;
use pipewire::node::Node;
use pipewire::proxy::Listener as ProxyListener;
use pipewire::types::ObjectType;

use crate::filterchain::{bus_args, hardware_strip_args, virtual_strip_args, LoadedModule};
use crate::links::LinkManager;
use crate::meter::MeterTap;
use crate::params;
use crate::registry::{parse, GraphModel};

pub struct EngineHandle {
    pub cmd_tx: pipewire::channel::Sender<EngineCommand>,
    pub evt_rx: mpsc::Receiver<EngineEvent>,
}

pub fn spawn() -> EngineHandle {
    let (cmd_tx, cmd_rx) = pipewire::channel::channel::<EngineCommand>();
    let (evt_tx, evt_rx) = mpsc::channel::<EngineEvent>();
    std::thread::Builder::new()
        .name("lm-engine".into())
        .spawn(move || {
            if let Err(e) = run(cmd_rx, evt_tx) {
                tracing::error!("engine thread died: {e}");
            }
        })
        .expect("spawn engine thread");
    EngineHandle { cmd_tx, evt_rx }
}

struct Engine {
    context: ContextRc,
    core: CoreRc,
    evt_tx: mpsc::Sender<EngineEvent>,
    app: AppState,
    model: GraphModel,
    links: LinkManager,
    strip_modules: HashMap<StripId, LoadedModule>,
    bus_modules: HashMap<BusId, LoadedModule>,
    strip_taps: Vec<MeterTap>,
    bus_taps: Vec<MeterTap>,
    /// Control-node name -> bound proxy (Props target for volume/mute/DSP).
    nodes: HashMap<String, Node>,
    /// Names we want to bind when they appear.
    wanted_controls: HashSet<String>,
    /// Keep metadata proxy + listener alive.
    metadata: Option<(Metadata, Box<dyn ProxyListener>)>,
    default_sink: Option<String>,
    default_source: Option<String>,
    built: bool,
    dirty: bool,
    seq: u32,
    /// Whether the 30 Hz drain emits. Off while the window is hidden.
    meters_on: bool,
}

impl Engine {
    fn send_state(&mut self) {
        self.refresh_devices();
        self.refresh_profiles();
        self.dirty = true;
        let _ = self.evt_tx.send(EngineEvent::State(self.app.clone()));
    }

    fn save_now(&mut self) {
        if !self.built {
            return;
        }
        let profile = crate::persist::Profile {
            take_default_output: self.app.take_default_output,
            strips: self.app.strips.clone(),
            buses: self.app.buses.clone(),
        };
        if let Err(e) = crate::persist::save_profile(&self.app.profile_name, &profile) {
            tracing::warn!("profile save failed: {e}");
        }
        let _ = crate::persist::save_config(&crate::persist::Config {
            last_profile: self.app.profile_name.clone(),
        });
        self.dirty = false;
    }

    fn refresh_devices(&mut self) {
        let mut devices: Vec<DeviceInfo> = self
            .model
            .nodes
            .values()
            .filter(|n| {
                (n.media_class == "Audio/Sink" || n.media_class == "Audio/Source")
                    && (n.name.starts_with("alsa_") || n.name.starts_with("bluez_"))
            })
            .map(|n| DeviceInfo {
                node_name: n.name.clone(),
                description: n.description.clone(),
                media_class: n.media_class.clone(),
                serial: n.serial,
                channels: 2,
            })
            .collect();
        devices.sort_by(|a, b| a.node_name.cmp(&b.node_name));
        self.app.devices = devices;
    }

    fn refresh_profiles(&mut self) {
        let mut names = crate::persist::list_profiles();
        // The active profile belongs in the menu even before its first save.
        if !self.app.profile_name.is_empty() && !names.contains(&self.app.profile_name) {
            names.push(self.app.profile_name.clone());
            names.sort_by_key(|n| n.to_lowercase());
        }
        self.app.profiles = names;
    }

    /// Hardware strips are online iff their capture device exists right now.
    fn mark_hardware_online(&mut self) {
        for s in &mut self.app.strips {
            if s.kind == StripKind::Hardware {
                s.online = s.hw_key.as_deref().is_some_and(|k| self.model.nodes.values().any(|n| n.name == k));
            }
        }
    }

    // ---- graph construction -------------------------------------------------

    fn build_graph(&mut self) {
        if self.built {
            return;
        }
        let profile_name = crate::persist::load_config().last_profile;
        if let Some(p) = crate::persist::load_profile(&profile_name) {
            self.app.strips = p.strips;
            self.app.buses = p.buses;
            self.app.take_default_output = p.take_default_output;
            self.app.profile_name = profile_name;
            self.mark_hardware_online();
            self.finish_build();
            return;
        }
        let default_sink = self
            .default_sink
            .clone()
            .or_else(|| self.first_node("Audio/Sink"));
        let Some(default_sink) = default_sink else {
            tracing::warn!("no audio sink found yet; deferring graph build");
            return;
        };
        let default_source = self.default_source.clone().or_else(|| self.first_node("Audio/Source"));
        let second_sink = self
            .model
            .nodes
            .values()
            .filter(|n| n.media_class == "Audio/Sink" && n.name.starts_with("alsa_output") && n.name != default_sink)
            .map(|n| n.name.clone())
            .next();

        // Default mixer: safe routes only (mic never feeds speakers by default).
        let mut strips = Vec::new();
        let mut next_id: StripId = 1;
        if let Some(src) = &default_source {
            strips.push(StripState {
                id: next_id,
                kind: StripKind::Hardware,
                label: "Mic".into(),
                hw_key: Some(src.clone()),
                online: true,
                gain_db: 0.0,
                mute: false,
                solo: false,
                routes: routes(&[BusId::B1]),
                gate: GateParams::default(),
                comp: CompParams::default(),
                eq: EqParams::default(),
            });
            next_id += 1;
        }
        for (label, on) in [("System", vec![BusId::A1]), ("Music", vec![BusId::A1]), ("Chat", vec![BusId::A1, BusId::B1])] {
            strips.push(StripState {
                id: next_id,
                kind: StripKind::Virtual,
                label: label.into(),
                hw_key: None,
                online: true,
                gain_db: 0.0,
                mute: false,
                solo: false,
                routes: routes(&on),
                gate: GateParams::default(),
                comp: CompParams::default(),
                eq: EqParams::default(),
            });
            next_id += 1;
        }
        let buses = vec![
            bus(BusId::A1, "Speakers", Some(default_sink.clone())),
            bus(BusId::A2, "Headphones", Some(second_sink.unwrap_or(default_sink))),
            bus(BusId::B1, "Stream Mic", None),
            bus(BusId::B2, "Aux", None),
        ];
        self.app.strips = strips;
        self.app.buses = buses;
        self.app.profile_name = "Default".into();
        self.finish_build();
    }

    /// Loads modules, routes, and taps for whatever `self.app` describes.
    fn finish_build(&mut self) {
        // Load modules.
        for strip in self.app.strips.clone() {
            self.load_strip_module(&strip);
        }
        for b in self.app.buses.clone() {
            self.load_bus_module(&b);
        }

        // Routing matrix.
        self.set_matrix_routes();

        // Meter taps (linked through the same reconciler).
        for strip in &self.app.strips {
            match MeterTap::new(self.core.clone(), &format!("s{}", strip.id)) {
                Ok(tap) => {
                    self.links.set_route(&strip.out_node(), &tap.node_name.clone(), true);
                    self.strip_taps.push(tap);
                }
                Err(e) => tracing::warn!("strip tap failed: {e}"),
            }
        }
        for b in &self.app.buses {
            match MeterTap::new(self.core.clone(), &format!("b{:?}", b.id).to_lowercase()) {
                Ok(tap) => {
                    self.links.set_route(&b.tap_node(), &tap.node_name.clone(), true);
                    self.bus_taps.push(tap);
                }
                Err(e) => tracing::warn!("bus tap failed: {e}"),
            }
        }

        self.built = true;
        self.reconcile();
        if self.app.take_default_output {
            self.apply_default_output(true);
        }
        self.send_state();
        self.save_now(); // ensure the profile file exists from first launch
        tracing::info!("graph built: {} strips, {} buses", self.app.strips.len(), self.app.buses.len());
    }

    fn load_strip_module(&mut self, strip: &StripState) {
        let args = match (&strip.kind, &strip.hw_key) {
            (StripKind::Hardware, Some(hw)) => hardware_strip_args(&strip.node_base(), &strip.label, hw),
            _ => virtual_strip_args(&strip.node_base(), &strip.label, false),
        };
        match LoadedModule::load(&self.context, "libpipewire-module-filter-chain", &args) {
            Ok(m) => {
                self.strip_modules.insert(strip.id, m);
            }
            Err(e) => tracing::error!("strip {} failed to load: {e}", strip.label),
        }
        self.wanted_controls.insert(strip.control_node());
    }

    fn load_bus_module(&mut self, b: &BusState) {
        let args = bus_args(&b.node_base(), &b.label, b.target_hw_key.as_deref());
        match LoadedModule::load(&self.context, "libpipewire-module-filter-chain", &args) {
            Ok(m) => {
                self.bus_modules.insert(b.id, m);
            }
            Err(e) => tracing::error!("bus {:?} failed to load: {e}", b.id),
        }
        self.wanted_controls.insert(b.in_node());
    }

    /// Tear down and re-create a strip's filter-chain (e.g. new capture
    /// device). Node names stay identical, so the LinkManager's desired routes
    /// and the meter tap re-link automatically as the new ports appear.
    fn reload_strip(&mut self, id: StripId) {
        let Some(strip) = self.app.strips.iter().find(|s| s.id == id).cloned() else { return };
        self.nodes.remove(&strip.control_node());
        self.strip_modules.remove(&id); // drop unloads the old module
        self.load_strip_module(&strip);
        self.reconcile();
    }

    fn reload_bus(&mut self, id: BusId) {
        let Some(b) = self.app.buses.iter().find(|b| b.id == id).cloned() else { return };
        self.nodes.remove(&b.in_node());
        self.bus_modules.remove(&id);
        self.load_bus_module(&b);
        self.reconcile();
    }

    // ---- profiles -----------------------------------------------------------

    /// Switch to the saved profile `name`, saving the current one first.
    ///
    /// Same layout (see [`same_layout`]): every module, link and meter tap is
    /// kept and only parameters and routes change, so apps playing into the
    /// mixer never lose their device. Otherwise the graph is torn down and
    /// rebuilt exactly as at startup, which apps hear as a device vanishing.
    fn switch_profile(&mut self, name: &str) {
        if !self.built || name == self.app.profile_name {
            return;
        }
        let Some(p) = crate::persist::load_profile(name) else {
            tracing::warn!("profile {name} could not be loaded; staying on {}", self.app.profile_name);
            self.send_state(); // re-sync the frontend's optimistic menu
            return;
        };
        self.save_now();
        let was_default = self.app.take_default_output;

        if same_layout(&self.app.strips, &self.app.buses, &p.strips, &p.buses) {
            // Routes are rebuilt from the new matrix; modules and taps stay.
            for s in &self.app.strips {
                for b in &self.app.buses {
                    self.links.set_route(&s.out_node(), &b.in_node(), false);
                }
            }
            self.app.strips = p.strips;
            self.app.buses = p.buses;
            self.app.profile_name = name.to_string();
            self.mark_hardware_online();
            self.set_matrix_routes();
            self.reconcile();
            let controls: Vec<String> = self
                .app
                .strips
                .iter()
                .map(|s| s.control_node())
                .chain(self.app.buses.iter().map(|b| b.in_node()))
                .collect();
            for node in controls {
                self.apply_all_for(&node);
            }
            if p.take_default_output != was_default {
                self.apply_default_output(p.take_default_output);
            }
            tracing::info!("profile {name} applied in place");
            self.send_state();
            self.save_now(); // records the switch in config.toml
        } else {
            tracing::info!("profile {name} has a different layout; rebuilding the graph");
            if was_default && !p.take_default_output {
                self.apply_default_output(false);
            }
            self.teardown();
            self.app.strips = p.strips;
            self.app.buses = p.buses;
            self.app.take_default_output = p.take_default_output;
            self.app.profile_name = name.to_string();
            self.mark_hardware_online();
            self.finish_build(); // modules, routes, taps, default output, state, save
        }
    }

    /// Unload every module, meter tap and link so `finish_build` can start
    /// from nothing. Node names are reused by the rebuild, so stale proxies
    /// must go too or params would be sent to dead nodes.
    fn teardown(&mut self) {
        self.built = false; // pauses the meter drain and autosave meanwhile
        self.links.clear();
        self.strip_taps.clear();
        self.bus_taps.clear();
        self.strip_modules.clear();
        self.bus_modules.clear();
        self.nodes.clear();
        self.wanted_controls.clear();
    }

    /// Desired strip -> bus links for the current routing matrix.
    fn set_matrix_routes(&mut self) {
        for strip in &self.app.strips {
            for (bus_id, on) in &strip.routes {
                if *on {
                    if let Some(b) = self.app.buses.iter().find(|b| b.id == *bus_id) {
                        self.links.set_route(&strip.out_node(), &b.in_node(), true);
                    }
                }
            }
        }
    }

    fn first_node(&self, class: &str) -> Option<String> {
        self.model
            .nodes
            .values()
            .filter(|n| n.media_class == class && n.name.starts_with("alsa_"))
            .map(|n| n.name.clone())
            .next()
    }

    fn reconcile(&mut self) {
        let Engine { links, core, model, .. } = self;
        links.reconcile(core, model);
    }

    // ---- param application --------------------------------------------------

    fn set_props(&self, node_name: &str, bytes: &[u8]) {
        let Some(node) = self.nodes.get(node_name) else { return };
        if let Some(pod) = libspa::pod::Pod::from_bytes(bytes) {
            node.set_param(libspa::param::ParamType::Props, 0, pod);
        }
    }

    fn apply_gain(&self, target: Target) {
        let (node, db) = match target {
            Target::Strip { strip } => match self.app.strips.iter().find(|s| s.id == strip) {
                Some(s) => (s.control_node(), s.gain_db),
                None => return,
            },
            Target::Bus { bus } => match self.app.buses.iter().find(|b| b.id == bus) {
                Some(b) => (b.in_node(), b.gain_db),
                None => return,
            },
        };
        let lin = if db == f32::NEG_INFINITY || db <= -71.0 { 0.0 } else { params::db_to_linear(db) };
        self.set_props(&node, &params::channel_volumes(lin));
    }

    /// Mutes are solo-aware: any active solo silences every non-solo strip.
    fn apply_strip_mutes(&self) {
        let any_solo = self.app.strips.iter().any(|s| s.solo);
        for s in &self.app.strips {
            let effective = s.mute || (any_solo && !s.solo);
            self.set_props(&s.control_node(), &params::mute(effective));
        }
    }

    fn apply_bus_mute(&self, bus: BusId) {
        if let Some(b) = self.app.buses.iter().find(|b| b.id == bus) {
            self.set_props(&b.in_node(), &params::mute(b.mute));
        }
    }

    fn apply_gate(&self, strip: &StripState) {
        let g = &strip.gate;
        let pairs: Vec<(&str, f32)> = vec![
            ("gate:enabled", g.enabled as u32 as f32),
            ("gate:gt", params::db_to_linear(g.threshold_db)),
            ("gate:at", g.attack_ms),
            ("gate:rt", g.release_ms),
            ("gate:hold", g.hold_ms),
        ];
        self.set_props(&strip.control_node(), &params::filter_params(&pairs));
    }

    fn apply_comp(&self, strip: &StripState) {
        let c = &strip.comp;
        let pairs: Vec<(&str, f32)> = vec![
            ("comp:enabled", c.enabled as u32 as f32),
            ("comp:al", params::db_to_linear(c.threshold_db)),
            ("comp:cr", c.ratio),
            ("comp:at", c.attack_ms),
            ("comp:rt", c.release_ms),
            ("comp:mk", params::db_to_linear(c.makeup_db)),
        ];
        self.set_props(&strip.control_node(), &params::filter_params(&pairs));
    }

    fn apply_limiter(&self, bus: &BusState) {
        self.set_props(&bus.in_node(), &params::filter_params(&params::limiter_pairs(&bus.limiter)));
    }

    fn apply_delay(&self, bus: &BusState) {
        let s = clamp_delay_ms(bus.delay_ms) / 1000.0;
        self.set_props(&bus.in_node(), &params::filter_params(&[("dl_l:Delay (s)", s), ("dl_r:Delay (s)", s)]));
    }

    fn apply_eq(&self, strip: &StripState) {
        self.apply_eq_to(&strip.control_node(), &strip.eq);
    }

    /// Buses run the same para_equalizer under the same "eq" node name, so
    /// the Props keys are identical; only the target node differs.
    fn apply_bus_eq(&self, bus: &BusState) {
        self.apply_eq_to(&bus.in_node(), &bus.eq);
    }

    fn apply_eq_to(&self, node_name: &str, e: &EqParams) {
        let mut pairs: Vec<(String, f32)> = vec![("eq:enabled".into(), e.enabled as u32 as f32)];
        for (i, band) in e.bands.iter().enumerate().take(8) {
            pairs.push((format!("eq:ft_{i}"), band.kind.lsp_filter_type()));
            pairs.push((format!("eq:f_{i}"), band.freq_hz.clamp(10.0, 24000.0)));
            pairs.push((format!("eq:g_{i}"), params::db_to_linear(band.gain_db)));
            pairs.push((format!("eq:q_{i}"), band.q.clamp(0.0, 100.0)));
        }
        let borrowed: Vec<(&str, f32)> = pairs.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        self.set_props(node_name, &params::filter_params(&borrowed));
    }

    /// Push the full parameter set for whichever strip/bus owns `node_name`.
    fn apply_all_for(&self, node_name: &str) {
        if let Some(s) = self.app.strips.iter().find(|s| s.control_node() == node_name) {
            self.apply_gain(Target::Strip { strip: s.id });
            self.apply_gate(s);
            self.apply_comp(s);
            self.apply_eq(s);
            self.apply_strip_mutes();
        }
        if let Some(b) = self.app.buses.iter().find(|b| b.in_node() == node_name) {
            self.apply_gain(Target::Bus { bus: b.id });
            self.apply_bus_eq(b);
            self.apply_limiter(b);
            self.apply_delay(b);
            self.apply_bus_mute(b.id);
        }
    }

    // ---- command handling ---------------------------------------------------

    fn handle(&mut self, cmd: EngineCommand) {
        match cmd {
            EngineCommand::SetGain { target, db } => {
                match target {
                    Target::Strip { strip } => {
                        if let Some(s) = self.app.strips.iter_mut().find(|s| s.id == strip) {
                            s.gain_db = db;
                        }
                    }
                    Target::Bus { bus } => {
                        if let Some(b) = self.app.buses.iter_mut().find(|b| b.id == bus) {
                            b.gain_db = db;
                        }
                    }
                }
                self.apply_gain(target);
                // No state event: gains stream at rAF rate and the frontend is
                // optimistic. Still mark dirty so the debounced save picks it up.
                self.dirty = true;
            }
            EngineCommand::SetMute { target, mute } => {
                match target {
                    Target::Strip { strip } => {
                        if let Some(s) = self.app.strips.iter_mut().find(|s| s.id == strip) {
                            s.mute = mute;
                        }
                        self.apply_strip_mutes();
                    }
                    Target::Bus { bus } => {
                        if let Some(b) = self.app.buses.iter_mut().find(|b| b.id == bus) {
                            b.mute = mute;
                        }
                        self.apply_bus_mute(bus);
                    }
                }
                self.send_state();
            }
            EngineCommand::SetSolo { strip, solo } => {
                if let Some(s) = self.app.strips.iter_mut().find(|s| s.id == strip) {
                    s.solo = solo;
                }
                self.apply_strip_mutes();
                self.send_state();
            }
            EngineCommand::SetRoute { strip, bus, on } => {
                let pair = self
                    .app
                    .strips
                    .iter_mut()
                    .find(|s| s.id == strip)
                    .map(|s| {
                        s.routes.insert(bus, on);
                        s.out_node()
                    })
                    .zip(self.app.buses.iter().find(|b| b.id == bus).map(|b| b.in_node()));
                if let Some((out, into)) = pair {
                    self.links.set_route(&out, &into, on);
                    self.reconcile();
                }
                self.send_state();
            }
            EngineCommand::SetGateParams { strip, params } => {
                if let Some(s) = self.app.strips.iter_mut().find(|s| s.id == strip) {
                    s.gate = params;
                }
                if let Some(s) = self.app.strips.iter().find(|s| s.id == strip) {
                    self.apply_gate(s);
                }
            }
            EngineCommand::SetCompParams { strip, params } => {
                if let Some(s) = self.app.strips.iter_mut().find(|s| s.id == strip) {
                    s.comp = params;
                }
                if let Some(s) = self.app.strips.iter().find(|s| s.id == strip) {
                    self.apply_comp(s);
                }
            }
            EngineCommand::SetEqParams { strip, params } => {
                tracing::debug!(
                    "SetEqParams strip={strip} enabled={} gains={:?}",
                    params.enabled,
                    params.bands.iter().map(|b| b.gain_db).collect::<Vec<_>>()
                );
                if let Some(s) = self.app.strips.iter_mut().find(|s| s.id == strip) {
                    s.eq = params;
                }
                if let Some(s) = self.app.strips.iter().find(|s| s.id == strip) {
                    self.apply_eq(s);
                }
                self.dirty = true;
            }
            EngineCommand::SetLimiterParams { bus, params } => {
                if let Some(b) = self.app.buses.iter_mut().find(|b| b.id == bus) {
                    b.limiter = params;
                }
                if let Some(b) = self.app.buses.iter().find(|b| b.id == bus) {
                    self.apply_limiter(b);
                }
            }
            EngineCommand::SetBusDelay { bus, ms } => {
                if let Some(b) = self.app.buses.iter_mut().find(|b| b.id == bus) {
                    b.delay_ms = clamp_delay_ms(ms);
                }
                if let Some(b) = self.app.buses.iter().find(|b| b.id == bus) {
                    self.apply_delay(b);
                }
                self.dirty = true;
            }
            EngineCommand::SetBusEqParams { bus, params } => {
                if let Some(b) = self.app.buses.iter_mut().find(|b| b.id == bus) {
                    b.eq = params;
                }
                if let Some(b) = self.app.buses.iter().find(|b| b.id == bus) {
                    self.apply_bus_eq(b);
                }
                self.dirty = true;
            }
            EngineCommand::SetStripDevice { strip, hw_key } => {
                let known = self.model.node_by_name(&hw_key).is_some();
                if let Some(s) = self.app.strips.iter_mut().find(|s| s.id == strip) {
                    if s.kind != StripKind::Hardware {
                        return;
                    }
                    s.hw_key = Some(hw_key);
                    s.online = known;
                }
                self.reload_strip(strip);
                self.send_state();
            }
            EngineCommand::SetBusTarget { bus, hw_key } => {
                if let Some(b) = self.app.buses.iter_mut().find(|b| b.id == bus) {
                    if b.target_hw_key.is_none() {
                        return; // B buses are virtual sources; no target to set
                    }
                    b.target_hw_key = Some(hw_key);
                }
                self.reload_bus(bus);
                self.send_state();
            }
            EngineCommand::SetDefaultOutput { on } => {
                self.apply_default_output(on);
                self.send_state();
            }
            EngineCommand::SetMetersEnabled { on } => {
                // The taps keep accumulating while disabled — peak is a latched
                // max — so discard whatever piled up rather than opening with a
                // stale spike from minutes ago.
                if on && !self.meters_on {
                    self.meter_frame();
                }
                self.meters_on = on;
            }
            EngineCommand::LoadProfile { name } => self.switch_profile(&name),
            EngineCommand::SaveProfileAs { name } => {
                if !crate::persist::valid_profile_name(&name) {
                    tracing::warn!("refusing profile name {name:?}");
                    return;
                }
                self.save_now(); // flush the profile we are leaving
                self.app.profile_name = name;
                self.save_now(); // write the copy and record it in config.toml
                self.send_state();
            }
            EngineCommand::RenameProfile { from, to } => {
                let active = from == self.app.profile_name;
                if active {
                    self.save_now(); // the file on disk must be current before it moves
                }
                match crate::persist::rename_profile(&from, &to) {
                    Ok(()) if active => {
                        self.app.profile_name = to;
                        self.save_now();
                    }
                    Ok(()) => {}
                    Err(e) => tracing::warn!("rename profile {from:?} -> {to:?} failed: {e}"),
                }
                self.send_state();
            }
            EngineCommand::DeleteProfile { name } => {
                if name == self.app.profile_name {
                    tracing::warn!("refusing to delete the active profile {name:?}");
                } else if let Err(e) = crate::persist::delete_profile(&name) {
                    tracing::warn!("delete profile {name:?} failed: {e}");
                }
                self.send_state();
            }
            EngineCommand::Shutdown => unreachable!("handled by caller"),
        }
    }

    fn apply_default_output(&mut self, on: bool) {
        let Some((md, _)) = &self.metadata else {
            tracing::warn!("no default metadata bound; cannot set default output");
            return;
        };
        if on {
            let Some(system) = self
                .app
                .strips
                .iter()
                .find(|s| s.kind == StripKind::Virtual)
                .map(|s| s.node_base())
            else {
                return;
            };
            md.set_property(
                0,
                "default.configured.audio.sink",
                Some("Spa:String:JSON"),
                Some(&format!("{{\"name\":\"{system}\"}}")),
            );
            tracing::info!("default output -> {system}");
        } else {
            // Restore the remembered hardware default, or clear so the
            // session manager picks the best available device.
            match &self.default_sink {
                Some(hw) => md.set_property(
                    0,
                    "default.configured.audio.sink",
                    Some("Spa:String:JSON"),
                    Some(&format!("{{\"name\":\"{hw}\"}}")),
                ),
                None => md.set_property(0, "default.configured.audio.sink", None, None),
            }
            tracing::info!("default output restored to {:?}", self.default_sink);
        }
        self.app.take_default_output = on;
    }

    // ---- metering -----------------------------------------------------------

    fn meter_frame(&mut self) -> lm_protocol::MeterFrame {
        self.seq = self.seq.wrapping_add(1);
        lm_protocol::MeterFrame {
            seq: self.seq,
            strips: self.strip_taps.iter().map(|t| t.accum.drain()).collect(),
            buses: self.bus_taps.iter().map(|t| t.accum.drain()).collect(),
        }
    }
}

fn routes(on: &[BusId]) -> std::collections::BTreeMap<BusId, bool> {
    BusId::ALL.iter().map(|b| (*b, on.contains(b))).collect()
}

fn bus(id: BusId, label: &str, target: Option<String>) -> BusState {
    BusState {
        id,
        label: label.into(),
        target_hw_key: target,
        online: true,
        gain_db: 0.0,
        mute: false,
        limiter: LimiterParams::default(),
        delay_ms: 0.0,
        eq: EqParams::default(),
    }
}

/// Whether two mixers can share one live graph: the same strips (id, kind,
/// capture device, in order — meter frames are indexed by position) and the
/// same buses (id, output device). Labels, levels, DSP and routes may differ;
/// those are applied as params. Anything else needs modules reloaded.
fn same_layout(a_strips: &[StripState], a_buses: &[BusState], b_strips: &[StripState], b_buses: &[BusState]) -> bool {
    a_strips.len() == b_strips.len()
        && a_buses.len() == b_buses.len()
        && a_strips.iter().zip(b_strips).all(|(a, b)| a.id == b.id && a.kind == b.kind && a.hw_key == b.hw_key)
        && a_buses.iter().zip(b_buses).all(|(a, b)| a.id == b.id && a.target_hw_key == b.target_hw_key)
}

/// NaN (or anything else non-finite) becomes 0 rather than poisoning the
/// delay line or the saved profile.
pub(crate) fn clamp_delay_ms(ms: f32) -> f32 {
    if ms.is_finite() { ms.clamp(0.0, lm_protocol::MAX_BUS_DELAY_MS) } else { 0.0 }
}

fn run(
    cmd_rx: pipewire::channel::Receiver<EngineCommand>,
    evt_tx: mpsc::Sender<EngineEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    pipewire::init();
    let mainloop = MainLoopRc::new(None)?;
    let context = ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let engine = Rc::new(RefCell::new(Engine {
        context: context.clone(),
        core: core.clone(),
        evt_tx,
        app: AppState::default(),
        model: GraphModel::default(),
        links: LinkManager::default(),
        strip_modules: HashMap::new(),
        bus_modules: HashMap::new(),
        strip_taps: Vec::new(),
        bus_taps: Vec::new(),
        nodes: HashMap::new(),
        wanted_controls: HashSet::new(),
        metadata: None,
        default_sink: None,
        default_source: None,
        built: false,
        dirty: false,
        seq: 0,
        meters_on: true,
    }));

    // --- registry events -> model + binding + reconcile ---
    let _reg_listener = {
        let engine = engine.clone();
        let engine_rm = engine.clone();
        let registry_weak = registry.downgrade();
        registry
            .add_listener_local()
            .global(move |global| {
                let mut e = engine.borrow_mut();
                match global.type_ {
                    ObjectType::Node => {
                        let Some(props) = global.props else { return };
                        let Some(n) = parse::node(global.id, props) else { return };
                        let name = n.name.clone();
                        let is_device = n.media_class == "Audio/Sink" || n.media_class == "Audio/Source";
                        e.model.nodes.insert(global.id, n);
                        if e.wanted_controls.contains(&name) && !e.nodes.contains_key(&name) {
                            if let Some(registry) = registry_weak.upgrade() {
                                if let Ok(node) = registry.bind::<Node, _>(global) {
                                    e.nodes.insert(name.clone(), node);
                                    e.apply_all_for(&name);
                                }
                            }
                        }
                        // Hardware strip target reappeared -> back online.
                        let mut changed = false;
                        for s in &mut e.app.strips {
                            if s.hw_key.as_deref() == Some(name.as_str()) && !s.online {
                                s.online = true;
                                changed = true;
                            }
                        }
                        if e.built && (is_device || changed) {
                            e.send_state();
                        }
                    }
                    ObjectType::Port => {
                        let Some(props) = global.props else { return };
                        if let Some(p) = parse::port(global.id, props) {
                            e.model.ports.insert(global.id, p);
                            if e.built {
                                e.reconcile();
                            }
                        }
                    }
                    ObjectType::Link => {
                        let Some(props) = global.props else { return };
                        if let Some(l) = parse::link(global.id, props) {
                            e.model.links.insert(global.id, l);
                        }
                    }
                    ObjectType::Metadata => {
                        let Some(props) = global.props else { return };
                        if props.get("metadata.name") != Some("default") || e.metadata.is_some() {
                            return;
                        }
                        let Some(registry) = registry_weak.upgrade() else { return };
                        if let Ok(md) = registry.bind::<Metadata, _>(global) {
                            let engine2 = engine.clone();
                            let listener = md
                                .add_listener_local()
                                .property(move |_subject, key, _type, value| {
                                    let mut e = engine2.borrow_mut();
                                    let name = value.and_then(parse_default_name);
                                    // Ignore our own nodes: while we ARE the
                                    // default, keep remembering the hardware
                                    // device underneath (A1 target, restore).
                                    if name.as_deref().is_some_and(|n| n.starts_with("lm.")) {
                                        return 0;
                                    }
                                    match key {
                                        Some("default.audio.sink") => e.default_sink = name,
                                        Some("default.audio.source") => e.default_source = name,
                                        _ => {}
                                    }
                                    0
                                })
                                .register();
                            e.metadata = Some((md, Box::new(listener)));
                        }
                    }
                    _ => {}
                }
            })
            .global_remove(move |id| {
                let mut e = engine_rm.borrow_mut();
                let removed_name = e.model.nodes.get(&id).map(|n| n.name.clone());
                e.model.remove_global(id);
                {
                    let Engine { links, model, .. } = &mut *e;
                    links.prune_dead(model);
                }
                if let Some(name) = removed_name {
                    // Drop stale proxies so the node re-binds if it reappears
                    // (device replug, strip/bus module reload).
                    e.nodes.remove(&name);
                    let mut changed = false;
                    for s in &mut e.app.strips {
                        if s.hw_key.as_deref() == Some(name.as_str()) {
                            s.online = false;
                            changed = true;
                        }
                    }
                    if e.built {
                        if changed {
                            tracing::info!("device {name} disappeared");
                        }
                        e.send_state();
                    }
                }
            })
            .register()
    };

    // --- initial enumeration done -> build graph ---
    let _core_listener = {
        let engine = engine.clone();
        core.add_listener_local()
            .done(move |_id, _seq| {
                engine.borrow_mut().build_graph();
            })
            .register()
    };
    core.sync(0)?;

    // --- commands from the Tauri side ---
    let _attached = {
        let engine = engine.clone();
        let ml = mainloop.clone();
        cmd_rx.attach(mainloop.loop_(), move |cmd| {
            if matches!(cmd, EngineCommand::Shutdown) {
                engine.borrow_mut().save_now();
                ml.quit();
                return;
            }
            engine.borrow_mut().handle(cmd);
        })
    };

    // --- 30 Hz meter drain ---
    let timer = {
        let engine = engine.clone();
        mainloop.loop_().add_timer(move |_| {
            let mut e = engine.borrow_mut();
            if !e.built || !e.meters_on {
                return;
            }
            let frame = e.meter_frame();
            let _ = e.evt_tx.send(EngineEvent::Meters(frame));
        })
    };
    timer
        .update_timer(Some(Duration::from_millis(33)), Some(Duration::from_millis(33)))
        .into_result()?;

    // Debounced profile auto-save.
    let save_timer = {
        let engine = engine.clone();
        mainloop.loop_().add_timer(move |_| {
            let mut e = engine.borrow_mut();
            if e.built && e.dirty {
                e.save_now();
            }
        })
    };
    save_timer
        .update_timer(Some(Duration::from_secs(5)), Some(Duration::from_secs(5)))
        .into_result()?;

    tracing::info!("engine running");
    mainloop.run();
    tracing::info!("engine stopped");
    Ok(())
}

/// Metadata values look like `{"name":"alsa_output.foo"}`.
fn parse_default_name(value: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()?
        .get("name")?
        .as_str()
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(id: StripId, kind: StripKind, hw: Option<&str>) -> StripState {
        StripState {
            id,
            kind,
            label: format!("Strip {id}"),
            hw_key: hw.map(String::from),
            online: true,
            gain_db: 0.0,
            mute: false,
            solo: false,
            routes: routes(&[BusId::A1]),
            gate: GateParams::default(),
            comp: CompParams::default(),
            eq: EqParams::default(),
        }
    }

    fn layout() -> (Vec<StripState>, Vec<BusState>) {
        (
            vec![strip(1, StripKind::Hardware, Some("alsa_input.mic")), strip(2, StripKind::Virtual, None)],
            vec![bus(BusId::A1, "Speakers", Some("alsa_output.a".into())), bus(BusId::B1, "Mic", None)],
        )
    }

    /// The common case — same rig, different settings — must switch live.
    #[test]
    fn settings_routes_and_labels_do_not_change_the_layout() {
        let (s, b) = layout();
        let (mut s2, mut b2) = layout();
        s2[0].gain_db = -12.0;
        s2[0].mute = true;
        s2[1].label = "Game".into();
        s2[1].routes = routes(&[BusId::A1, BusId::B1]);
        s2[1].eq.enabled = true;
        b2[0].delay_ms = 40.0;
        b2[0].limiter.enabled = false;
        assert!(same_layout(&s, &b, &s2, &b2));
    }

    /// Anything a module is built from forces a rebuild.
    #[test]
    fn devices_kinds_ids_and_counts_change_the_layout() {
        let (s, b) = layout();
        let mutations: Vec<(&str, Box<dyn Fn(&mut Vec<StripState>, &mut Vec<BusState>)>)> = vec![
            ("capture device", Box::new(|s, _| s[0].hw_key = Some("alsa_input.other".into()))),
            ("strip kind", Box::new(|s, _| s[1].kind = StripKind::Hardware)),
            ("strip id", Box::new(|s, _| s[1].id = 9)),
            ("extra strip", Box::new(|s, _| s.push(strip(3, StripKind::Virtual, None)))),
            ("strip order", Box::new(|s, _| s.swap(0, 1))),
            ("bus output device", Box::new(|_, b| b[0].target_hw_key = Some("alsa_output.b".into()))),
            ("bus removed", Box::new(|_, b| { b.pop(); })),
        ];
        for (what, mutate) in mutations {
            let (mut s2, mut b2) = layout();
            mutate(&mut s2, &mut b2);
            assert!(!same_layout(&s, &b, &s2, &b2), "a different {what} must rebuild");
        }
    }
}
