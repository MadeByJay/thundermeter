//! Shared types between the audio engine, the Tauri shell, and (via serde)
//! the frontend. Serialization is camelCase to match the TypeScript mirror in
//! `src/lib/types.ts` — keep the two in sync.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BusId {
    A1,
    A2,
    B1,
    B2,
}

impl BusId {
    pub const ALL: [BusId; 4] = [BusId::A1, BusId::A2, BusId::B1, BusId::B2];

    /// Lowercase node-name fragment: `lm.bus.a1`.
    pub fn node_base(&self) -> String {
        format!("lm.bus.{}", format!("{self:?}").to_lowercase())
    }
}

pub type StripId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripKind {
    /// Appears as an Audio/Sink other apps can play into.
    Virtual,
    /// Captures from a hardware (or any existing) source node.
    Hardware,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateParams {
    pub enabled: bool,
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub hold_ms: f32,
}

impl Default for GateParams {
    fn default() -> Self {
        Self { enabled: false, threshold_db: -40.0, attack_ms: 20.0, release_ms: 100.0, hold_ms: 50.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompParams {
    pub enabled: bool,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
}

impl Default for CompParams {
    fn default() -> Self {
        Self { enabled: false, threshold_db: -18.0, ratio: 4.0, attack_ms: 20.0, release_ms: 100.0, makeup_db: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqBandKind {
    LowShelf,
    Peak,
    HighShelf,
    /// Removes everything below the frequency ("low cut").
    HighPass,
    /// Removes everything above the frequency ("high cut").
    LowPass,
    /// A narrow cut at the frequency (hum, a ringing resonance).
    Notch,
    /// Keeps only a range around the frequency.
    BandPass,
}

impl EqBandKind {
    /// LSP para_equalizer `ft_N` filter-type value.
    pub fn lsp_filter_type(&self) -> f32 {
        match self {
            EqBandKind::Peak => 1.0,      // Bell
            EqBandKind::HighPass => 2.0,  // Hi-pass
            EqBandKind::HighShelf => 3.0, // Hi-shelf
            EqBandKind::LowPass => 4.0,   // Lo-pass
            EqBandKind::LowShelf => 5.0,  // Lo-shelf
            EqBandKind::Notch => 6.0,     // Notch
            EqBandKind::BandPass => 9.0,  // Bandpass
        }
    }

    /// Whether the band has a gain at all. Pass and notch filters only have a
    /// frequency and Q; their `gain_db` is ignored.
    pub fn has_gain(&self) -> bool {
        matches!(self, EqBandKind::LowShelf | EqBandKind::Peak | EqBandKind::HighShelf)
    }

    /// Whether `EqBand::slope` applies (the cut filters).
    pub fn has_slope(&self) -> bool {
        matches!(self, EqBandKind::HighPass | EqBandKind::LowPass)
    }
}

/// Highest `EqBand::slope` value: LSP's "x4", 48 dB/octave.
pub const MAX_EQ_SLOPE: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    pub kind: EqBandKind,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    /// Steepness of a high/low-pass band: 0..=MAX_EQ_SLOPE = 12/24/36/48 dB
    /// per octave (LSP `s_N` "x1".."x4"). Ignored by other kinds. `default`
    /// so profiles written before filters existed still load.
    #[serde(default)]
    pub slope: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqParams {
    pub enabled: bool,
    pub bands: Vec<EqBand>,
}

impl EqParams {
    /// Bands 4..8 are reserved for the voice-color pad (voice-tuned corners),
    /// independent of the four user-facing EQ-panel bands.
    pub fn color_defaults() -> [EqBand; 4] {
        [
            EqBand { kind: EqBandKind::LowShelf, freq_hz: 300.0, gain_db: 0.0, q: 0.7, slope: 0 },
            EqBand { kind: EqBandKind::Peak, freq_hz: 400.0, gain_db: 0.0, q: 0.8, slope: 0 },
            EqBand { kind: EqBandKind::Peak, freq_hz: 3000.0, gain_db: 0.0, q: 0.8, slope: 0 },
            EqBand { kind: EqBandKind::HighShelf, freq_hz: 3500.0, gain_db: 0.0, q: 0.7, slope: 0 },
        ]
    }
}

impl Default for EqParams {
    fn default() -> Self {
        let mut bands = vec![
            EqBand { kind: EqBandKind::LowShelf, freq_hz: 100.0, gain_db: 0.0, q: 0.7, slope: 0 },
            EqBand { kind: EqBandKind::Peak, freq_hz: 400.0, gain_db: 0.0, q: 1.0, slope: 0 },
            EqBand { kind: EqBandKind::Peak, freq_hz: 2500.0, gain_db: 0.0, q: 1.0, slope: 0 },
            EqBand { kind: EqBandKind::HighShelf, freq_hz: 8000.0, gain_db: 0.0, q: 0.7, slope: 0 },
        ];
        bands.extend(Self::color_defaults());
        Self { enabled: false, bands }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimiterParams {
    pub enabled: bool,
    pub threshold_db: f32,
    pub release_ms: f32,
}

impl Default for LimiterParams {
    fn default() -> Self {
        Self { enabled: true, threshold_db: -1.0, release_ms: 50.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripState {
    pub id: StripId,
    pub kind: StripKind,
    pub label: String,
    /// Stable PipeWire identity (`node.name`) of the hardware source, for Hardware strips.
    pub hw_key: Option<String>,
    pub online: bool,
    pub gain_db: f32,
    pub mute: bool,
    pub solo: bool,
    pub routes: BTreeMap<BusId, bool>,
    pub gate: GateParams,
    pub comp: CompParams,
    /// `default` so profiles written before the EQ existed still load.
    #[serde(default)]
    pub eq: EqParams,
}

impl StripState {
    /// Base node name; virtual strips' sink node carries this name directly.
    pub fn node_base(&self) -> String {
        format!("lm.strip.{}", self.id)
    }

    /// Node that carries Props (volume/mute/filter params): the capture side.
    pub fn control_node(&self) -> String {
        match self.kind {
            StripKind::Virtual => self.node_base(),
            StripKind::Hardware => format!("{}.cap", self.node_base()),
        }
    }

    /// Node whose output ports feed the routing matrix.
    pub fn out_node(&self) -> String {
        format!("{}.out", self.node_base())
    }
}

/// Upper bound of a bus's output delay. The filter-chain allocates its delay
/// line for this length up front, so raising it costs memory on every bus.
pub const MAX_BUS_DELAY_MS: f32 = 500.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BusState {
    pub id: BusId,
    pub label: String,
    /// `node.name` of the hardware sink this bus feeds (A buses). None = virtual source (B buses).
    pub target_hw_key: Option<String>,
    pub online: bool,
    pub gain_db: f32,
    pub mute: bool,
    pub limiter: LimiterParams,
    /// Post-limiter output delay, for lining up devices with different
    /// latency (VoiceMeeter's per-bus delay). `default` so profiles written
    /// before it existed still load, at 0.
    #[serde(default)]
    pub delay_ms: f32,
    /// Pre-limiter EQ shaping everything this bus sends. Same 8-band layout
    /// as a strip so the plugin, Props keys and panel are shared, but only
    /// bands 0–3 are user-facing: buses have no color pad, so 4–7 stay flat.
    /// `default` so profiles written before bus EQ still load.
    #[serde(default)]
    pub eq: EqParams,
}

impl BusState {
    pub fn node_base(&self) -> String {
        self.id.node_base()
    }

    /// Mix-input node (also carries Props for gain/mute/limiter).
    pub fn in_node(&self) -> String {
        format!("{}.in", self.node_base())
    }

    /// Node whose output ports carry the post-limiter signal (meter tap point).
    pub fn tap_node(&self) -> String {
        if self.target_hw_key.is_some() {
            format!("{}.out", self.node_base())
        } else {
            self.node_base()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub node_name: String,
    pub description: String,
    pub media_class: String,
    pub serial: u64,
    pub channels: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub strips: Vec<StripState>,
    pub buses: Vec<BusState>,
    pub devices: Vec<DeviceInfo>,
    pub profile_name: String,
    /// Every saved profile name, sorted for the menu; includes `profile_name`.
    pub profiles: Vec<String>,
    /// True while linuxmeeter's System strip is the system default output.
    pub take_default_output: bool,
}

/// One batched meter update, ~30/s. Channel layout: [peakL, peakR, rmsL, rmsR]
/// in dBFS; entries follow `AppState.strips` / `AppState.buses` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterFrame {
    pub seq: u32,
    pub strips: Vec<[f32; 4]>,
    pub buses: Vec<[f32; 4]>,
}

/// A strip or a bus, as sent by the frontend: `{"strip": 3}` or `{"bus": "A1"}`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    Strip { strip: StripId },
    Bus { bus: BusId },
}

/// Commands into the engine thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineCommand {
    SetGain { target: Target, db: f32 },
    SetMute { target: Target, mute: bool },
    SetSolo { strip: StripId, solo: bool },
    SetRoute { strip: StripId, bus: BusId, on: bool },
    SetGateParams { strip: StripId, params: GateParams },
    SetCompParams { strip: StripId, params: CompParams },
    SetEqParams { strip: StripId, params: EqParams },
    SetLimiterParams { bus: BusId, params: LimiterParams },
    /// Delay a bus's output by `ms` (clamped to `0..=MAX_BUS_DELAY_MS`).
    SetBusDelay { bus: BusId, ms: f32 },
    SetBusEqParams { bus: BusId, params: EqParams },
    /// Point a hardware strip at a different capture device (module reload).
    SetStripDevice { strip: StripId, hw_key: String },
    /// Point an A-bus at a different output device (module reload).
    SetBusTarget { bus: BusId, hw_key: String },
    /// Make the System strip the system-wide default output (VoiceMeeter
    /// style: all desktop audio flows through the mixer), or restore the
    /// previous hardware default.
    SetDefaultOutput { on: bool },
    /// Start/stop the 30 Hz meter drain. Every frame costs a script evaluation
    /// in the webview, so a mixer sitting in the tray stops paying for meters
    /// nobody can see. Audio is unaffected either way.
    SetMetersEnabled { on: bool },
    /// Switch to a saved profile. The current one is saved first. If both have
    /// the same strips and bus devices the new settings are applied to the
    /// live graph (no audio drop); otherwise the graph is rebuilt.
    LoadProfile { name: String },
    /// Save the current mixer under a new name and make that the active profile.
    SaveProfileAs { name: String },
    /// Rename a saved profile (the active one included). Refuses to overwrite.
    RenameProfile { from: String, to: String },
    /// Delete a saved profile. The active profile cannot be deleted.
    DeleteProfile { name: String },
    Shutdown,
}

/// Events out of the engine thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineEvent {
    /// Full snapshot; sent on startup and after any state-affecting change.
    State(AppState),
    Meters(MeterFrame),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(id: StripId, kind: StripKind) -> StripState {
        StripState {
            id,
            kind,
            label: "Test".into(),
            hw_key: None,
            online: true,
            gain_db: 0.0,
            mute: false,
            solo: false,
            routes: BusId::ALL.iter().map(|b| (*b, false)).collect(),
            gate: GateParams::default(),
            comp: CompParams::default(),
            eq: EqParams::default(),
        }
    }

    fn bus(id: BusId, target: Option<&str>) -> BusState {
        BusState {
            id,
            label: "Test".into(),
            target_hw_key: target.map(String::from),
            online: true,
            gain_db: 0.0,
            mute: false,
            limiter: LimiterParams::default(),
            delay_ms: 0.0,
            eq: EqParams::default(),
        }
    }

    // ---- node naming: the identity profiles, routes, and taps all key on ----

    #[test]
    fn virtual_strip_controls_its_sink_node_directly() {
        let s = strip(3, StripKind::Virtual);
        assert_eq!(s.node_base(), "lm.strip.3");
        assert_eq!(s.control_node(), "lm.strip.3", "the sink node carries Props");
        assert_eq!(s.out_node(), "lm.strip.3.out");
    }

    /// A hardware strip's Props live on the capture side, not the base name —
    /// writing to the wrong node makes every fader and DSP knob a no-op.
    #[test]
    fn hardware_strip_controls_its_capture_node() {
        let s = strip(1, StripKind::Hardware);
        assert_eq!(s.control_node(), "lm.strip.1.cap");
        assert_eq!(s.out_node(), "lm.strip.1.out");
    }

    #[test]
    fn bus_node_names_are_lowercase() {
        assert_eq!(BusId::A1.node_base(), "lm.bus.a1");
        assert_eq!(BusId::B2.node_base(), "lm.bus.b2");
    }

    /// A buses meter after their output node; B buses ARE the source node the
    /// world sees, so that is where the post-limiter signal is tapped.
    #[test]
    fn bus_tap_point_depends_on_bus_kind() {
        assert_eq!(bus(BusId::A1, Some("alsa_output.hdmi")).tap_node(), "lm.bus.a1.out");
        assert_eq!(bus(BusId::B1, None).tap_node(), "lm.bus.b1");
        assert_eq!(bus(BusId::A1, Some("x")).in_node(), "lm.bus.a1.in");
    }

    #[test]
    fn strip_node_names_are_unique_per_strip() {
        let a = strip(1, StripKind::Virtual);
        let b = strip(2, StripKind::Virtual);
        assert_ne!(a.out_node(), b.out_node());
        assert_ne!(a.control_node(), b.control_node());
    }

    // ---- IPC contract with src/lib/types.ts --------------------------------

    /// `Target` is untagged: the frontend sends `{"strip": 3}` / `{"bus": "A1"}`.
    /// If this drifts, every command silently fails to deserialize.
    #[test]
    fn target_serializes_untagged() {
        assert_eq!(serde_json::to_string(&Target::Strip { strip: 3 }).unwrap(), r#"{"strip":3}"#);
        assert_eq!(serde_json::to_string(&Target::Bus { bus: BusId::A1 }).unwrap(), r#"{"bus":"A1"}"#);

        assert_eq!(
            serde_json::from_str::<Target>(r#"{"strip":7}"#).unwrap(),
            Target::Strip { strip: 7 }
        );
        assert_eq!(
            serde_json::from_str::<Target>(r#"{"bus":"B2"}"#).unwrap(),
            Target::Bus { bus: BusId::B2 }
        );
    }

    /// The TypeScript mirror expects camelCase. These are the exact keys
    /// `src/lib/types.ts` reads.
    #[test]
    fn app_state_uses_camel_case_keys() {
        let json = serde_json::to_value(AppState {
            strips: vec![strip(1, StripKind::Hardware)],
            buses: vec![bus(BusId::A1, Some("alsa_output.hdmi"))],
            devices: vec![DeviceInfo {
                node_name: "alsa_output.hdmi".into(),
                description: "HDMI".into(),
                media_class: "Audio/Sink".into(),
                serial: 42,
                channels: 2,
            }],
            profile_name: "Default".into(),
            profiles: vec!["Default".into(), "Gaming".into()],
            take_default_output: true,
        })
        .expect("serialize");

        assert!(json.get("profileName").is_some());
        assert_eq!(json["profiles"], serde_json::json!(["Default", "Gaming"]));
        assert!(json.get("takeDefaultOutput").is_some());
        assert!(json.get("profile_name").is_none(), "snake_case would break the frontend");

        let s = &json["strips"][0];
        for key in ["hwKey", "gainDb", "kind", "routes", "gate", "comp", "eq"] {
            assert!(s.get(key).is_some(), "strip is missing {key}");
        }
        assert!(json["buses"][0].get("targetHwKey").is_some());
        assert!(json["buses"][0].get("delayMs").is_some());
        assert!(json["buses"][0].get("eq").is_some());
        assert!(json["devices"][0].get("nodeName").is_some());
        assert!(json["devices"][0].get("mediaClass").is_some());
    }

    /// StripKind is snake_case on the wire: types.ts declares "virtual" | "hardware".
    #[test]
    fn strip_kind_is_lowercase_on_the_wire() {
        assert_eq!(serde_json::to_value(StripKind::Virtual).unwrap(), "virtual");
        assert_eq!(serde_json::to_value(StripKind::Hardware).unwrap(), "hardware");
    }

    #[test]
    fn eq_band_kind_matches_the_typescript_union() {
        assert_eq!(serde_json::to_value(EqBandKind::LowShelf).unwrap(), "low_shelf");
        assert_eq!(serde_json::to_value(EqBandKind::Peak).unwrap(), "peak");
        assert_eq!(serde_json::to_value(EqBandKind::HighShelf).unwrap(), "high_shelf");
        assert_eq!(serde_json::to_value(EqBandKind::HighPass).unwrap(), "high_pass");
        assert_eq!(serde_json::to_value(EqBandKind::LowPass).unwrap(), "low_pass");
        assert_eq!(serde_json::to_value(EqBandKind::Notch).unwrap(), "notch");
        assert_eq!(serde_json::to_value(EqBandKind::BandPass).unwrap(), "band_pass");
    }

    /// Bands saved before filters existed have no `slope`; they must load as
    /// the gentlest one rather than failing the whole profile.
    #[test]
    fn eq_band_without_slope_loads_as_12_db_per_octave() {
        let b: EqBand = serde_json::from_str(r#"{"kind":"peak","freqHz":400,"gainDb":-3,"q":1}"#).unwrap();
        assert_eq!(b.slope, 0);
        assert_eq!(serde_json::to_value(b).unwrap()["slope"], 0, "and is written back out");
    }

    #[test]
    fn only_shelves_and_peaks_have_gain_and_only_cuts_have_slope() {
        use EqBandKind::*;
        for k in [LowShelf, Peak, HighShelf] {
            assert!(k.has_gain() && !k.has_slope(), "{k:?}");
        }
        for k in [HighPass, LowPass] {
            assert!(!k.has_gain() && k.has_slope(), "{k:?}");
        }
        for k in [Notch, BandPass] {
            assert!(!k.has_gain() && !k.has_slope(), "{k:?}");
        }
    }

    #[test]
    fn dsp_params_use_camel_case_keys() {
        let gate = serde_json::to_value(GateParams::default()).unwrap();
        for key in ["enabled", "thresholdDb", "attackMs", "releaseMs", "holdMs"] {
            assert!(gate.get(key).is_some(), "gate is missing {key}");
        }
        let comp = serde_json::to_value(CompParams::default()).unwrap();
        for key in ["thresholdDb", "ratio", "attackMs", "releaseMs", "makeupDb"] {
            assert!(comp.get(key).is_some(), "comp is missing {key}");
        }
        let lim = serde_json::to_value(LimiterParams::default()).unwrap();
        for key in ["enabled", "thresholdDb", "releaseMs"] {
            assert!(lim.get(key).is_some(), "limiter is missing {key}");
        }
    }

    #[test]
    fn meter_frame_round_trips() {
        let f = MeterFrame { seq: 9, strips: vec![[-6.0, -6.5, -9.0, -9.2]], buses: vec![[0.0; 4]] };
        let back: MeterFrame = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    /// Commands are externally tagged; this is what src-tauri forwards.
    #[test]
    fn engine_commands_round_trip() {
        let cmds = vec![
            EngineCommand::SetGain { target: Target::Strip { strip: 1 }, db: -6.0 },
            EngineCommand::SetRoute { strip: 1, bus: BusId::B1, on: true },
            EngineCommand::SetEqParams { strip: 2, params: EqParams::default() },
            EngineCommand::SetBusDelay { bus: BusId::A2, ms: 42.0 },
            EngineCommand::SetBusEqParams { bus: BusId::A1, params: EqParams::default() },
            EngineCommand::SetDefaultOutput { on: true },
            EngineCommand::SetMetersEnabled { on: false },
            EngineCommand::LoadProfile { name: "Gaming".into() },
            EngineCommand::SaveProfileAs { name: "Late night".into() },
            EngineCommand::RenameProfile { from: "Late night".into(), to: "Night".into() },
            EngineCommand::DeleteProfile { name: "Night".into() },
            EngineCommand::Shutdown,
        ];
        for cmd in cmds {
            let text = serde_json::to_string(&cmd).expect("serialize");
            serde_json::from_str::<EngineCommand>(&text).expect("deserialize");
        }
    }

    // ---- DSP defaults -----------------------------------------------------

    /// Four user-facing EQ bands plus four reserved for the color pad. The pad
    /// drives indices 4..7 by number, so the count is load-bearing.
    #[test]
    fn eq_defaults_to_eight_bands_split_four_and_four() {
        let eq = EqParams::default();
        assert_eq!(eq.bands.len(), 8);
        assert_eq!(&eq.bands[4..], &EqParams::color_defaults()[..]);
        assert!(!eq.enabled);
    }

    #[test]
    fn eq_panel_bands_span_the_spectrum() {
        let eq = EqParams::default();
        assert_eq!(eq.bands[0].kind, EqBandKind::LowShelf);
        assert_eq!(eq.bands[3].kind, EqBandKind::HighShelf);
        let freqs: Vec<f32> = eq.bands[..4].iter().map(|b| b.freq_hz).collect();
        assert!(freqs.windows(2).all(|w| w[0] < w[1]), "bands must ascend: {freqs:?}");
    }

    /// These are LSP para_equalizer `ft_N` port values; a wrong number silently
    /// selects the wrong filter shape.
    #[test]
    fn lsp_filter_types_are_the_documented_values() {
        assert_eq!(EqBandKind::Peak.lsp_filter_type(), 1.0);
        assert_eq!(EqBandKind::HighShelf.lsp_filter_type(), 3.0);
        assert_eq!(EqBandKind::LowShelf.lsp_filter_type(), 5.0);
        // From para_equalizer_x8_stereo.ttl: Hi-pass 2, Lo-pass 4, Notch 6, Bandpass 9.
        assert_eq!(EqBandKind::HighPass.lsp_filter_type(), 2.0);
        assert_eq!(EqBandKind::LowPass.lsp_filter_type(), 4.0);
        assert_eq!(EqBandKind::Notch.lsp_filter_type(), 6.0);
        assert_eq!(EqBandKind::BandPass.lsp_filter_type(), 9.0);
    }

    /// The limiter is the last thing before hardware and the virtual mic, so it
    /// defaults to on with headroom.
    #[test]
    fn limiter_defaults_to_enabled_with_headroom() {
        let l = LimiterParams::default();
        assert!(l.enabled);
        assert!(l.threshold_db < 0.0);
    }

    #[test]
    fn gate_and_comp_default_to_bypassed() {
        assert!(!GateParams::default().enabled);
        assert!(!CompParams::default().enabled);
    }

    #[test]
    fn bus_ids_cover_two_hardware_and_two_virtual() {
        assert_eq!(BusId::ALL.len(), 4);
        assert_eq!(BusId::ALL, [BusId::A1, BusId::A2, BusId::B1, BusId::B2]);
    }
}
