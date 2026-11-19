import { describe, expect, it } from "vitest";
import { EQ_FMAX, EQ_FMIN, freqToX, responseCurve, xToFreq } from "./eq";
import type { EqBand } from "../types";

const band = (over: Partial<EqBand> = {}): EqBand => ({
  kind: "peak",
  freqHz: 1000,
  gainDb: 0,
  q: 1,
  slope: 0,
  ...over,
});

/** dB of the summed response at the frequency closest to `f`. */
function dbAt(bands: EqBand[], f: number, points = 512): number {
  const curve = responseCurve(bands, points);
  let best = curve[0]!;
  for (const p of curve) {
    if (Math.abs(Math.log(p.f / f)) < Math.abs(Math.log(best.f / f))) best = p;
  }
  return best.db;
}

describe("frequency axis", () => {
  it("maps the audible range onto the full width", () => {
    expect(freqToX(EQ_FMIN, 100)).toBeCloseTo(0, 9);
    expect(freqToX(EQ_FMAX, 100)).toBeCloseTo(100, 9);
  });

  it("is logarithmic — each decade takes equal width", () => {
    const decade1 = freqToX(200, 900) - freqToX(20, 900);
    const decade2 = freqToX(2000, 900) - freqToX(200, 900);
    expect(decade2).toBeCloseTo(decade1, 6);
  });

  it("round-trips position and frequency", () => {
    for (const f of [20, 100, 440, 1000, 5000, 20000]) {
      expect(xToFreq(freqToX(f, 640), 640)).toBeCloseTo(f, 3);
    }
  });
});

describe("EQ response curve", () => {
  it("is flat when every band is at 0 dB", () => {
    const curve = responseCurve([band(), band({ freqHz: 100 }), band({ freqHz: 8000 })], 128);
    for (const p of curve) expect(Math.abs(p.db)).toBeLessThan(0.01);
  });

  it("spans the audible range and returns the requested resolution", () => {
    const curve = responseCurve([band()], 200);
    expect(curve).toHaveLength(200);
    expect(curve[0]!.f).toBeCloseTo(EQ_FMIN, 6);
    expect(curve[199]!.f).toBeCloseTo(EQ_FMAX, 6);
  });

  it("puts a peak band's boost at its centre frequency", () => {
    const bands = [band({ freqHz: 1000, gainDb: 6, q: 2 })];
    expect(dbAt(bands, 1000)).toBeCloseTo(6, 1);
    // and leaves distant frequencies alone
    expect(Math.abs(dbAt(bands, 60))).toBeLessThan(0.5);
    expect(Math.abs(dbAt(bands, 15000))).toBeLessThan(0.5);
  });

  it("cuts as well as boosts", () => {
    expect(dbAt([band({ freqHz: 1000, gainDb: -9, q: 2 })], 1000)).toBeCloseTo(-9, 1);
  });

  it("makes a low shelf lift the bottom and leave the top", () => {
    const bands = [band({ kind: "low_shelf", freqHz: 200, gainDb: 6, q: 0.7 })];
    expect(dbAt(bands, 25)).toBeCloseTo(6, 0);
    expect(Math.abs(dbAt(bands, 15000))).toBeLessThan(0.5);
  });

  it("makes a high shelf lift the top and leave the bottom", () => {
    const bands = [band({ kind: "high_shelf", freqHz: 4000, gainDb: 6, q: 0.7 })];
    expect(dbAt(bands, 18000)).toBeCloseTo(6, 0);
    expect(Math.abs(dbAt(bands, 30))).toBeLessThan(0.5);
  });

  /// The displayed curve is the sum of the bands, which is what makes the EQ
  /// panel's overlay meaningful.
  it("sums overlapping bands", () => {
    const two = [band({ freqHz: 1000, gainDb: 4, q: 1 }), band({ freqHz: 1000, gainDb: 3, q: 1 })];
    expect(dbAt(two, 1000)).toBeCloseTo(7, 1);
  });

  it("narrows the affected range as Q rises", () => {
    const wide = [band({ freqHz: 1000, gainDb: 12, q: 0.5 })];
    const narrow = [band({ freqHz: 1000, gainDb: 12, q: 8 })];
    expect(dbAt(wide, 1000)).toBeCloseTo(12, 0);
    expect(dbAt(narrow, 1000)).toBeCloseTo(12, 0);
    // An octave away the wide band still does a lot; the narrow one barely.
    expect(dbAt(wide, 2000)).toBeGreaterThan(dbAt(narrow, 2000) + 3);
  });

  it("survives extreme frequencies without producing NaN", () => {
    for (const f of [10, 19, 20000, 24000]) {
      const curve = responseCurve([band({ freqHz: f, gainDb: 6 })], 64);
      for (const p of curve) expect(Number.isFinite(p.db)).toBe(true);
    }
  });
});

// Expected values are what the LSP plugin itself produced for a 1 kHz tone on
// a live graph (tests/audio.rs `bus_filters_cut_where_the_panel_says` and the
// characterisation behind lspPassQ). The panel must draw what you will hear.
describe("filter bands match the measured plugin", () => {
  const hp = (over: Partial<EqBand> = {}) => band({ kind: "high_pass", freqHz: 2000, q: 0.707, ...over });

  it("high-pass an octave above costs ~12 dB at x1 and four times that at x4", () => {
    expect(dbAt([hp()], 1000)).toBeCloseTo(-11.7, 0);
    expect(dbAt([hp({ slope: 3 })], 1000)).toBeGreaterThan(-49.7);
    expect(dbAt([hp({ slope: 3 })], 1000)).toBeLessThan(-43.7);
  });

  it("uses LSP's effective Q: q 4 resonates +8 dB at the cutoff, not +12", () => {
    expect(dbAt([hp({ freqHz: 1000, q: 4 })], 1000, 2048)).toBeCloseTo(8.0, 0);
    expect(dbAt([hp({ freqHz: 1000, q: 0.5 })], 1000, 2048)).toBeCloseTo(-2.5, 0);
  });

  it("mirrors for low-pass", () => {
    expect(dbAt([band({ kind: "low_pass", freqHz: 500, q: 0.707 })], 1000)).toBeCloseTo(-11.6, 0);
    expect(dbAt([band({ kind: "low_pass", freqHz: 8000, q: 0.707, slope: 3 })], 1000)).toBeCloseTo(0, 0);
  });

  it("notches deep at the centre with the measured width", () => {
    expect(dbAt([band({ kind: "notch", freqHz: 1000, q: 4 })], 1000, 2048)).toBeLessThan(-20);
    expect(dbAt([band({ kind: "notch", freqHz: 1189, q: 2 })], 1000, 2048)).toBeCloseTo(-6.7, 0);
  });

  it("draws LSP's band-pass as a fixed band from f/4 to 4f", () => {
    const bp = (freqHz: number, q = 1) => [band({ kind: "band_pass", freqHz, q })];
    expect(dbAt(bp(1000), 1000, 2048)).toBeCloseTo(-0.5, 0);
    expect(dbAt(bp(4000), 1000, 2048)).toBeCloseTo(-3.0, 0);
    expect(dbAt(bp(8000), 1000, 2048)).toBeCloseTo(-7.0, 0);
    expect(dbAt(bp(8000, 8), 1000, 2048)).toBeCloseTo(dbAt(bp(8000, 0.5), 1000, 2048), 5); // Q is ignored
  });

  it("ignores gain on gainless kinds and slope on the rest", () => {
    for (const kind of ["high_pass", "low_pass", "notch", "band_pass"] as const) {
      expect(dbAt([band({ kind, freqHz: 3000, gainDb: 12 })], 1500)).toBeCloseTo(dbAt([band({ kind, freqHz: 3000 })], 1500), 5);
    }
    const peak = band({ freqHz: 1000, gainDb: 6 });
    expect(dbAt([{ ...peak, slope: 3 }], 1200)).toBeCloseTo(dbAt([peak], 1200), 5);
  });
});
