/** Mirrors crates/lm-protocol (serde-renamed to camelCase at the IPC boundary). */

export type BusId = "A1" | "A2" | "B1" | "B2";
export const BUS_IDS: BusId[] = ["A1", "A2", "B1", "B2"];

export type StripKind = "virtual" | "hardware";

export interface GateParams {
  enabled: boolean;
  thresholdDb: number;
  attackMs: number;
  releaseMs: number;
  holdMs: number;
}

export interface CompParams {
  enabled: boolean;
  thresholdDb: number;
  ratio: number;
  attackMs: number;
  releaseMs: number;
  makeupDb: number;
}

export interface LimiterParams {
  enabled: boolean;
  thresholdDb: number;
  releaseMs: number;
}

export type EqBandKind = "low_shelf" | "peak" | "high_shelf" | "high_pass" | "low_pass" | "notch" | "band_pass";

/** Mirrors EqBandKind::has_gain: pass and notch filters ignore gainDb. */
export const hasGain = (k: EqBandKind): boolean => k === "low_shelf" || k === "peak" || k === "high_shelf";
/** Mirrors EqBandKind::has_slope: only the cut filters use `slope`. */
export const hasSlope = (k: EqBandKind): boolean => k === "high_pass" || k === "low_pass";
/** Mirrors lm_protocol::MAX_EQ_SLOPE (x4 = 48 dB/oct). */
export const MAX_EQ_SLOPE = 3;

export interface EqBand {
  kind: EqBandKind;
  freqHz: number;
  gainDb: number;
  q: number;
  /** High/low-pass steepness, 0..MAX_EQ_SLOPE = 12/24/36/48 dB per octave. */
  slope: number;
}

export interface EqParams {
  enabled: boolean;
  bands: EqBand[];
}

export interface StripState {
  id: number;
  kind: StripKind;
  label: string;
  /** Stable PipeWire identity (node.name) for hardware strips. */
  hwKey: string | null;
  online: boolean;
  gainDb: number;
  mute: boolean;
  solo: boolean;
  routes: Record<BusId, boolean>;
  gate: GateParams;
  comp: CompParams;
  eq: EqParams;
}

export interface BusState {
  id: BusId;
  label: string;
  targetHwKey: string | null;
  online: boolean;
  gainDb: number;
  mute: boolean;
  limiter: LimiterParams;
  /** Post-limiter output delay in ms, 0..MAX_BUS_DELAY_MS. */
  delayMs: number;
  /** Pre-limiter EQ. Same 8-band shape as a strip; only bands 0–3 are
   *  user-facing (buses have no color pad), 4–7 stay flat. */
  eq: EqParams;
}

/** Mirrors lm_protocol::MAX_BUS_DELAY_MS — the engine clamps to it too. */
export const MAX_BUS_DELAY_MS = 500;

export interface DeviceInfo {
  nodeName: string;
  description: string;
  mediaClass: string;
  serial: number;
  channels: number;
}

export interface AppState {
  strips: StripState[];
  buses: BusState[];
  devices: DeviceInfo[];
  profileName: string;
  /** Every saved profile, sorted for the menu; includes profileName. */
  profiles: string[];
  /** True while linuxmeeter's System strip is the system default output. */
  takeDefaultOutput: boolean;
}

/** One batched meter update, ~30/s. Channel layout: [peakL, peakR, rmsL, rmsR] in dBFS. */
export interface MeterFrame {
  seq: number;
  /** Keyed by strip id order in AppState.strips. */
  strips: number[][];
  /** Keyed by bus order in AppState.buses. */
  buses: number[][];
}

export type MeterKey = `s:${number}` | `b:${BusId}`;
