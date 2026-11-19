/** EQ frequency-response math (RBJ audio-EQ-cookbook biquads).
 *
 * Approximates the LSP para_equalizer curve closely enough for display; the
 * actual filtering happens in the plugin.
 */
import type { EqBand } from "../types";
import { MAX_EQ_SLOPE, hasGain, hasSlope } from "../types";

const FS = 48000;

interface Biquad {
  b0: number;
  b1: number;
  b2: number;
  a0: number;
  a1: number;
  a2: number;
}

/** LSP's RLC high/low-pass and notch filters match the textbook ones at
 *  this Q, not at the Q they are given: measured against the plugin on a
 *  live graph (tests/audio.rs `bus_filters_cut_where_the_panel_says`), the
 *  fit is within ~0.2 dB across q 0.5–4. Bells and shelves keep raw Q. */
export function lspPassQ(q: number): number {
  return 0.5 + Math.max(0, q) / 2;
}

/** Edges of LSP's band-pass: a fixed band (it ignores Q at slope x1, the
 *  only slope offered) with 6 dB/oct skirts below f/4 and above 4f. */
export const BANDPASS_SPAN = 4;

function coeffs(band: EqBand): Biquad {
  const A = Math.pow(10, band.gainDb / 40);
  const w0 = (2 * Math.PI * Math.min(band.freqHz, FS / 2 - 1)) / FS;
  const cos = Math.cos(w0);
  const sin = Math.sin(w0);
  const passKind = band.kind === "high_pass" || band.kind === "low_pass" || band.kind === "notch";
  const q = passKind ? lspPassQ(band.q) : Math.max(0.1, band.q);
  const alpha = sin / (2 * q);
  const sqA = Math.sqrt(A);

  switch (band.kind) {
    case "high_pass":
      return { b0: (1 + cos) / 2, b1: -(1 + cos), b2: (1 + cos) / 2, a0: 1 + alpha, a1: -2 * cos, a2: 1 - alpha };
    case "low_pass":
      return { b0: (1 - cos) / 2, b1: 1 - cos, b2: (1 - cos) / 2, a0: 1 + alpha, a1: -2 * cos, a2: 1 - alpha };
    case "notch":
      return { b0: 1, b1: -2 * cos, b2: 1, a0: 1 + alpha, a1: -2 * cos, a2: 1 - alpha };
    case "band_pass": // drawn by bandPassDb; unity here so it is never used by mistake
      return { b0: 1, b1: 0, b2: 0, a0: 1, a1: 0, a2: 0 };
    case "peak":
      return {
        b0: 1 + alpha * A,
        b1: -2 * cos,
        b2: 1 - alpha * A,
        a0: 1 + alpha / A,
        a1: -2 * cos,
        a2: 1 - alpha / A,
      };
    case "low_shelf":
      return {
        b0: A * (A + 1 - (A - 1) * cos + 2 * sqA * alpha),
        b1: 2 * A * (A - 1 - (A + 1) * cos),
        b2: A * (A + 1 - (A - 1) * cos - 2 * sqA * alpha),
        a0: A + 1 + (A - 1) * cos + 2 * sqA * alpha,
        a1: -2 * (A - 1 + (A + 1) * cos),
        a2: A + 1 + (A - 1) * cos - 2 * sqA * alpha,
      };
    case "high_shelf":
      return {
        b0: A * (A + 1 + (A - 1) * cos + 2 * sqA * alpha),
        b1: -2 * A * (A - 1 + (A + 1) * cos),
        b2: A * (A + 1 + (A - 1) * cos - 2 * sqA * alpha),
        a0: A + 1 - (A - 1) * cos + 2 * sqA * alpha,
        a1: 2 * (A - 1 - (A + 1) * cos),
        a2: A + 1 - (A - 1) * cos - 2 * sqA * alpha,
      };
  }
}

/** |H(e^jw)| in dB for one biquad at frequency f. */
function magDb(c: Biquad, f: number): number {
  const w = (2 * Math.PI * f) / FS;
  const cw = Math.cos(w);
  const c2w = Math.cos(2 * w);
  const sw = Math.sin(w);
  const s2w = Math.sin(2 * w);
  const nr = c.b0 + c.b1 * cw + c.b2 * c2w;
  const ni = -(c.b1 * sw + c.b2 * s2w);
  const dr = c.a0 + c.a1 * cw + c.a2 * c2w;
  const di = -(c.a1 * sw + c.a2 * s2w);
  const mag = Math.sqrt((nr * nr + ni * ni) / (dr * dr + di * di));
  return 20 * Math.log10(Math.max(1e-6, mag));
}

export const EQ_FMIN = 20;
export const EQ_FMAX = 20000;

export function freqToX(f: number, width: number): number {
  return (Math.log10(f / EQ_FMIN) / Math.log10(EQ_FMAX / EQ_FMIN)) * width;
}

export function xToFreq(x: number, width: number): number {
  return EQ_FMIN * Math.pow(EQ_FMAX / EQ_FMIN, x / width);
}

/** LSP band-pass: first-order high-pass at f/SPAN times first-order
 *  low-pass at f*SPAN (measured: -0.5 dB at f, -3 dB at f/4, -7 dB at f/8). */
function bandPassDb(center: number, f: number): number {
  const lo = f / (center / BANDPASS_SPAN);
  const hi = f / (center * BANDPASS_SPAN);
  const mag = lo / Math.sqrt(1 + lo * lo) / Math.sqrt(1 + hi * hi);
  return 20 * Math.log10(Math.max(1e-6, mag));
}

/** One band's response in dB at each frequency. A cut filter's slope
 *  cascades identical sections (x4 = four times the x1 response, as
 *  measured), and gainless kinds ignore whatever gainDb they carry. */
function bandResponse(band: EqBand): (f: number) => number {
  if (band.kind === "band_pass") return (f) => bandPassDb(band.freqHz, f);
  const c = coeffs(hasGain(band.kind) ? band : { ...band, gainDb: 0 });
  const sections = hasSlope(band.kind) ? 1 + Math.max(0, Math.min(MAX_EQ_SLOPE, Math.round(band.slope ?? 0))) : 1;
  return (f) => sections * magDb(c, f);
}

/** Summed response of all bands (gain 0 bands contribute ~nothing). */
export function responseCurve(bands: EqBand[], points: number): Array<{ f: number; db: number }> {
  const rs = bands.map(bandResponse);
  const out: Array<{ f: number; db: number }> = [];
  for (let i = 0; i < points; i++) {
    const f = EQ_FMIN * Math.pow(EQ_FMAX / EQ_FMIN, i / (points - 1));
    let db = 0;
    for (const r of rs) db += r(f);
    out.push({ f, db });
  }
  return out;
}
