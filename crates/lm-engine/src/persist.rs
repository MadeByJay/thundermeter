//! Profile persistence: TOML under `$XDG_CONFIG_HOME/thundermeter/`.
//!
//! `config.toml` holds app-level settings (currently the last profile name);
//! `profiles/<name>.toml` holds everything needed to rebuild the mixer.
//! Identity is by `node.name` — never transient PipeWire ids.

use std::fs;
use std::io;
use std::path::PathBuf;

use lm_protocol::{BusState, StripKind, StripState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub take_default_output: bool,
    pub strips: Vec<StripState>,
    pub buses: Vec<BusState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub last_profile: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { last_profile: "Default".into() }
    }
}

fn xdg_config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME not set")).join(".config"))
}

pub fn config_dir() -> PathBuf {
    xdg_config_home().join("thundermeter")
}

/// First launch after the rename from linuxmeeter: copy its settings across
/// so the mixer comes up exactly as it was. A copy, not a move, so upstream
/// linuxmeeter keeps working from its own folder. Does nothing once
/// `~/.config/thundermeter` exists.
pub fn migrate_from_linuxmeeter() {
    match copy_legacy_config(&xdg_config_home().join("linuxmeeter"), &config_dir()) {
        Ok(true) => tracing::info!("copied settings from ~/.config/linuxmeeter"),
        Ok(false) => {}
        Err(e) => tracing::warn!("could not copy linuxmeeter settings: {e}"),
    }
}

/// Copies `config.toml` and `profiles/*.toml` from `old` into a not-yet-
/// existing `new`. Returns whether anything was copied.
fn copy_legacy_config(old: &std::path::Path, new: &std::path::Path) -> io::Result<bool> {
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    fs::create_dir_all(new.join("profiles"))?;
    if old.join("config.toml").is_file() {
        fs::copy(old.join("config.toml"), new.join("config.toml"))?;
    }
    if let Ok(entries) = fs::read_dir(old.join("profiles")) {
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "toml") && path.is_file() {
                fs::copy(&path, new.join("profiles").join(e.file_name()))?;
            }
        }
    }
    Ok(true)
}

pub fn load_config() -> Config {
    let path = config_dir().join("config.toml");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).map_err(|e| tracing::warn!("bad config.toml: {e}")).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &Config) -> io::Result<()> {
    let toml = toml::to_string_pretty(config).map_err(io::Error::other)?;
    atomic_write(&config_dir().join("config.toml"), &toml)
}

pub fn profiles_dir() -> PathBuf {
    config_dir().join("profiles")
}

/// Longest profile name accepted, in characters.
pub const MAX_PROFILE_NAME: usize = 48;

/// Profile names arrive from the UI and become file names, so anything that
/// could escape `profiles/` or hide the file is refused. Mirrors
/// `validProfileName` in `src/lib/utils/profile.ts`.
pub fn valid_profile_name(name: &str) -> bool {
    let n = name.chars().count();
    n > 0
        && n <= MAX_PROFILE_NAME
        && name.trim() == name
        && !name.starts_with('.')
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control())
}

/// Every saved profile, sorted case-insensitively for the menu.
pub fn list_profiles() -> Vec<String> {
    list_profiles_in(&profiles_dir())
}

fn list_profiles_in(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            // `x.toml.tmp` (an interrupted atomic write) has extension "tmp".
            (path.extension()? == "toml").then(|| path.file_stem()?.to_str().map(String::from))?
        })
        .filter(|n| valid_profile_name(n))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn rename_profile(from: &str, to: &str) -> io::Result<()> {
    rename_profile_in(&profiles_dir(), from, to)
}

fn rename_profile_in(dir: &std::path::Path, from: &str, to: &str) -> io::Result<()> {
    if !valid_profile_name(from) || !valid_profile_name(to) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid profile name"));
    }
    let target = dir.join(format!("{to}.toml"));
    if target.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("profile {to} already exists")));
    }
    fs::rename(dir.join(format!("{from}.toml")), target)
}

pub fn delete_profile(name: &str) -> io::Result<()> {
    delete_profile_in(&profiles_dir(), name)
}

fn delete_profile_in(dir: &std::path::Path, name: &str) -> io::Result<()> {
    if !valid_profile_name(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid profile name"));
    }
    fs::remove_file(dir.join(format!("{name}.toml")))
}

pub fn load_profile(name: &str) -> Option<Profile> {
    if !valid_profile_name(name) {
        tracing::warn!("refusing to load profile with invalid name {name:?}");
        return None;
    }
    let path = profiles_dir().join(format!("{name}.toml"));
    let text = fs::read_to_string(&path).ok()?;
    match toml::from_str::<Profile>(&text) {
        Ok(mut p) => {
            sanitize(&mut p);
            tracing::info!("loaded profile {name} ({})", path.display());
            Some(p)
        }
        Err(e) => {
            tracing::warn!("profile {name} unreadable, using defaults: {e}");
            None
        }
    }
}

pub fn save_profile(name: &str, profile: &Profile) -> io::Result<()> {
    if !valid_profile_name(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid profile name"));
    }
    let dir = profiles_dir();
    fs::create_dir_all(&dir)?;
    let toml = toml::to_string_pretty(profile).map_err(io::Error::other)?;
    atomic_write(&dir.join(format!("{name}.toml")), &toml)
}

/// Reset transient runtime state that should never survive a restart.
fn sanitize(p: &mut Profile) {
    for s in &mut p.strips {
        s.solo = false;
        // `online` is recomputed against the live registry at build time.
        s.online = s.kind == StripKind::Virtual;
        // Migrate pre-color-pad profiles (4 bands) to 8 bands.
        while s.eq.bands.len() < 8 {
            let i = s.eq.bands.len() - 4;
            s.eq.bands.push(lm_protocol::EqParams::color_defaults()[i.min(3)]);
        }
    }
    for b in &mut p.buses {
        b.online = true;
        // Profiles are hand-editable; keep the delay inside the line's length.
        b.delay_ms = crate::engine::clamp_delay_ms(b.delay_ms);
        // A hand-trimmed band list would leave plugin bands at stale values;
        // pad from the defaults (flat) up to the full eight.
        let defaults = lm_protocol::EqParams::default().bands;
        while b.eq.bands.len() < defaults.len() {
            b.eq.bands.push(defaults[b.eq.bands.len()]);
        }
    }
}

fn atomic_write(path: &std::path::Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lm_protocol::{BusId, BusState, EqBand, EqBandKind, EqParams, GateParams, LimiterParams, StripState};
    use std::collections::BTreeMap;

    fn routes(on: &[BusId]) -> BTreeMap<BusId, bool> {
        BusId::ALL.iter().map(|b| (*b, on.contains(b))).collect()
    }

    fn strip(id: u32, kind: StripKind) -> StripState {
        StripState {
            id,
            kind,
            label: "Test".into(),
            hw_key: (kind == StripKind::Hardware).then(|| "alsa_input.usb-foo".to_string()),
            online: true,
            gain_db: -6.0,
            mute: false,
            solo: false,
            routes: routes(&[BusId::A1]),
            gate: GateParams::default(),
            comp: Default::default(),
            eq: EqParams::default(),
        }
    }

    fn profile(strips: Vec<StripState>) -> Profile {
        Profile {
            take_default_output: false,
            strips,
            buses: vec![BusState {
                id: BusId::A1,
                label: "Speakers".into(),
                target_hw_key: Some("alsa_output.hdmi".into()),
                online: false,
                gain_db: 0.0,
                mute: false,
                limiter: LimiterParams::default(),
                delay_ms: 0.0,
                eq: EqParams::default(),
            }],
        }
    }

    /// Solo is a live performance state. Restoring it would leave the user
    /// silently muted on every other strip after a restart.
    #[test]
    fn sanitize_clears_solo() {
        let mut s = strip(1, StripKind::Virtual);
        s.solo = true;
        let mut p = profile(vec![s]);
        sanitize(&mut p);
        assert!(!p.strips[0].solo);
    }

    /// A hardware strip is only online if its device is present, which is
    /// recomputed against the live registry — never trusted from disk.
    #[test]
    fn sanitize_resets_hardware_online_but_trusts_virtual() {
        let mut p = profile(vec![strip(1, StripKind::Hardware), strip(2, StripKind::Virtual)]);
        sanitize(&mut p);
        assert!(!p.strips[0].online, "hardware strips start offline until seen");
        assert!(p.strips[1].online, "virtual strips always exist");
        assert!(p.buses[0].online);
    }

    /// Profiles written before the color pad existed have only the four
    /// EQ-panel bands; bands 4..7 must be filled with the voice-tuned corners
    /// or the pad would drive whatever indices happen to exist.
    #[test]
    fn sanitize_migrates_pre_color_pad_profiles_to_eight_bands() {
        let mut s = strip(1, StripKind::Virtual);
        s.eq.bands.truncate(4);
        let mut p = profile(vec![s]);
        sanitize(&mut p);

        let bands = &p.strips[0].eq.bands;
        assert_eq!(bands.len(), 8);
        assert_eq!(&bands[4..], &EqParams::color_defaults()[..], "color bands must be the voice-tuned defaults");
    }

    #[test]
    fn sanitize_preserves_user_eq_bands_while_migrating() {
        let mut s = strip(1, StripKind::Virtual);
        s.eq.bands.truncate(4);
        s.eq.bands[1] = EqBand { kind: EqBandKind::Peak, freq_hz: 900.0, gain_db: -4.5, q: 2.0, slope: 0 };
        let mut p = profile(vec![s]);
        sanitize(&mut p);
        assert_eq!(p.strips[0].eq.bands[1].freq_hz, 900.0);
        assert_eq!(p.strips[0].eq.bands[1].gain_db, -4.5);
    }

    #[test]
    fn sanitize_leaves_current_profiles_untouched() {
        let mut p = profile(vec![strip(1, StripKind::Virtual)]);
        let before = p.strips[0].eq.bands.clone();
        sanitize(&mut p);
        assert_eq!(p.strips[0].eq.bands, before);
    }

    /// Profiles are advertised as hand-editable, so the TOML has to survive a
    /// full round trip with everything the mixer needs to rebuild itself.
    #[test]
    fn profile_survives_a_toml_round_trip() {
        let mut p = profile(vec![strip(1, StripKind::Hardware), strip(2, StripKind::Virtual)]);
        p.take_default_output = true;
        p.strips[0].gain_db = -13.5;
        p.strips[0].routes.insert(BusId::B1, true);
        p.strips[0].gate.enabled = true;
        p.strips[0].gate.threshold_db = -33.0;

        let text = toml::to_string_pretty(&p).expect("serialize");
        let back: Profile = toml::from_str(&text).expect("deserialize");

        assert!(back.take_default_output);
        assert_eq!(back.strips[0].gain_db, -13.5);
        assert_eq!(back.strips[0].routes[&BusId::B1], true);
        assert_eq!(back.strips[0].gate.threshold_db, -33.0);
        assert_eq!(back.strips[0].hw_key.as_deref(), Some("alsa_input.usb-foo"));
        assert_eq!(back.buses[0].target_hw_key.as_deref(), Some("alsa_output.hdmi"));
        assert_eq!(back.strips[1].kind, StripKind::Virtual);
    }

    /// A profile written before the EQ existed has no `eq` key at all. It must
    /// load with defaults rather than being rejected, which would silently
    /// reset the user's whole mixer.
    #[test]
    fn profile_predating_the_eq_still_loads() {
        const OLD: &str = r#"
take_default_output = false

[[strips]]
id = 1
kind = "virtual"
label = "System"
online = true
gainDb = -6.0
mute = false
solo = false

[strips.routes]
A1 = true
A2 = false
B1 = false
B2 = false

[strips.gate]
enabled = false
thresholdDb = -40.0
attackMs = 20.0
releaseMs = 100.0
holdMs = 50.0

[strips.comp]
enabled = false
thresholdDb = -18.0
ratio = 4.0
attackMs = 20.0
releaseMs = 100.0
makeupDb = 0.0

[[buses]]
id = "A1"
label = "Speakers"
targetHwKey = "alsa_output.hdmi"
online = true
gainDb = 0.0
mute = false

[buses.limiter]
enabled = true
thresholdDb = -1.0
releaseMs = 50.0
"#;
        let mut back: Profile = toml::from_str(OLD).expect("eq must be #[serde(default)]");
        assert_eq!(back.strips[0].label, "System");
        assert_eq!(back.strips[0].gain_db, -6.0);
        sanitize(&mut back);
        assert_eq!(back.strips[0].eq.bands.len(), 8, "defaults then migration give a full band set");
        assert_eq!(back.buses[0].delay_ms, 0.0, "profiles predating bus delay load undelayed");
        assert!(!back.buses[0].eq.enabled, "profiles predating bus EQ load with it bypassed");
        assert_eq!(back.buses[0].eq.bands.len(), 8);
        assert!(back.buses[0].eq.bands.iter().all(|b| b.gain_db == 0.0), "and flat");
    }

    #[test]
    fn sanitize_pads_a_trimmed_bus_eq_to_eight_flat_bands() {
        let mut p = profile(vec![]);
        p.buses[0].eq.bands.truncate(2);
        p.buses[0].eq.bands[0].gain_db = -3.0;
        sanitize(&mut p);
        let bands = &p.buses[0].eq.bands;
        assert_eq!(bands.len(), 8);
        assert_eq!(bands[0].gain_db, -3.0, "existing bands are kept");
        assert_eq!(&bands[2..], &EqParams::default().bands[2..]);
    }

    #[test]
    fn sanitize_keeps_bus_delay_inside_the_delay_line() {
        let mut p = profile(vec![]);
        for (written, loaded) in [(-20.0, 0.0), (35.5, 35.5), (9000.0, lm_protocol::MAX_BUS_DELAY_MS), (f32::NAN, 0.0)] {
            p.buses[0].delay_ms = written;
            sanitize(&mut p);
            assert_eq!(p.buses[0].delay_ms, loaded, "delayMs = {written} in the profile");
        }
    }

    /// A fresh, empty directory per test (tests run in parallel).
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("lm-persist-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Names become file names: nothing may climb out of profiles/, hide the
    /// file, or carry invisible characters.
    #[test]
    fn profile_names_cannot_escape_the_profiles_dir() {
        for ok in ["Default", "Gaming", "Stream (late)", "music-2", "Café"] {
            assert!(valid_profile_name(ok), "{ok:?} should be allowed");
        }
        let long = "x".repeat(MAX_PROFILE_NAME + 1);
        for bad in ["", " ", " lead", "trail ", ".hidden", "..", "../etc", "a/b", "a\\b", "tab\there", "nul\0", long.as_str()] {
            assert!(!valid_profile_name(bad), "{bad:?} should be refused");
        }
    }

    #[test]
    fn listing_shows_profiles_only_sorted_case_insensitively() {
        let dir = scratch_dir("list");
        for f in ["zeta.toml", "Alpha.toml", "beta.toml", "half.toml.tmp", "notes.txt", ".hidden.toml"] {
            fs::write(dir.join(f), "").unwrap();
        }
        assert_eq!(list_profiles_in(&dir), ["Alpha", "beta", "zeta"]);
        assert!(list_profiles_in(&dir.join("missing")).is_empty(), "no profiles dir yet is not an error");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_refuses_to_overwrite_and_delete_removes() {
        let dir = scratch_dir("rename");
        fs::write(dir.join("A.toml"), "a").unwrap();
        fs::write(dir.join("B.toml"), "b").unwrap();

        let err = rename_profile_in(&dir, "A", "B").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(dir.join("B.toml")).unwrap(), "b", "existing profile untouched");

        rename_profile_in(&dir, "A", "C").unwrap();
        assert_eq!(list_profiles_in(&dir), ["B", "C"]);
        assert_eq!(rename_profile_in(&dir, "C", "../x").unwrap_err().kind(), io::ErrorKind::InvalidInput);

        delete_profile_in(&dir, "B").unwrap();
        assert_eq!(list_profiles_in(&dir), ["C"]);
        assert_eq!(delete_profile_in(&dir, "../C").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_defaults_to_the_default_profile() {
        assert_eq!(Config::default().last_profile, "Default");
    }

    #[test]
    fn config_dir_lives_under_xdg_config_home() {
        // Uses whatever the environment says; assert the shape, not the value.
        let dir = config_dir();
        assert!(dir.ends_with("thundermeter"), "got {}", dir.display());
    }

    /// The first thundermeter launch picks up linuxmeeter's profiles; later
    /// launches never overwrite thundermeter's own settings with them.
    #[test]
    fn linuxmeeter_settings_are_copied_once() {
        let root = scratch_dir("migrate");
        let (old, new) = (root.join("linuxmeeter"), root.join("thundermeter"));
        fs::create_dir_all(old.join("profiles")).unwrap();
        fs::write(old.join("config.toml"), "last_profile = \"Gaming\"\n").unwrap();
        fs::write(old.join("profiles/Gaming.toml"), "g").unwrap();
        fs::write(old.join("profiles/half.toml.tmp"), "partial").unwrap();

        assert!(copy_legacy_config(&old, &new).unwrap());
        assert_eq!(fs::read_to_string(new.join("profiles/Gaming.toml")).unwrap(), "g");
        assert!(fs::read_to_string(new.join("config.toml")).unwrap().contains("Gaming"));
        assert!(!new.join("profiles/half.toml.tmp").exists(), "interrupted writes are not carried over");
        assert!(old.join("profiles/Gaming.toml").exists(), "a copy: linuxmeeter keeps its own settings");

        fs::write(new.join("profiles/Gaming.toml"), "changed in thundermeter").unwrap();
        assert!(!copy_legacy_config(&old, &new).unwrap(), "only on first launch");
        assert_eq!(fs::read_to_string(new.join("profiles/Gaming.toml")).unwrap(), "changed in thundermeter");
        assert!(!copy_legacy_config(&root.join("never-installed"), &root.join("fresh")).unwrap());
        fs::remove_dir_all(&root).unwrap();
    }
}
