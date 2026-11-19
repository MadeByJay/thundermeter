<script lang="ts">
  import type { EqBand, EqBandKind, EqParams } from "../lib/types";
  import { hasGain, hasSlope } from "../lib/types";
  import { EQ_FMAX, EQ_FMIN, freqToX, responseCurve, xToFreq } from "../lib/utils/eq";

  /** Owner-agnostic: strips and buses both open this panel. It edits bands
   *  0–3 only and carries 4–7 through untouched (a strip's color pad owns
   *  them; on a bus they stay flat). */
  interface Props {
    /** Shown beside "EQ" in the header: the strip or bus label. */
    title: string;
    eq: EqParams;
    onchange: (params: EqParams) => void;
    onclose: () => void;
  }
  let { title, eq, onchange, onclose }: Props = $props();

  const W = 396;
  const H = 180;
  const DB_RANGE = 18; // ±18 dB display

  import { onMount } from "svelte";

  let canvas: HTMLCanvasElement;
  let selected = $state(0);
  let dragging = $state(false);

  onMount(() => {
    // Untangle crossed bands from older sessions: keep kinds in place,
    // redistribute the frequencies in ascending order.
    const freqs = eq.bands.slice(0, 4).map((b) => b.freqHz);
    const sorted = [...freqs].sort((a, b) => a - b);
    if (freqs.some((f, i) => f !== sorted[i])) {
      const bands = eq.bands.map((b, i) => (i < 4 ? { ...b, freqHz: sorted[i]! } : b));
      onchange({ ...eq, bands });
    }
  });

  const KIND_LABEL: Record<EqBandKind, string> = {
    low_shelf: "LOW",
    peak: "PEAK",
    high_shelf: "HIGH",
    high_pass: "HP",
    low_pass: "LP",
    notch: "NOTCH",
    band_pass: "BAND",
  };
  const KIND_TITLE: Record<EqBandKind, string> = {
    low_shelf: "Low shelf: lift or cut everything below",
    peak: "Peak: lift or cut around the frequency",
    high_shelf: "High shelf: lift or cut everything above",
    high_pass: "High-pass (low cut): remove everything below",
    low_pass: "Low-pass (high cut): remove everything above",
    notch: "Notch: remove a narrow band (hum, ringing)",
    band_pass: "Band-pass: keep about two octaves either side of the frequency",
  };
  const KINDS = Object.keys(KIND_LABEL) as EqBandKind[];
  const SLOPES = [12, 24, 36, 48]; // dB/oct for slope 0..3
  /** The band the TYPE/SLOPE rows edit. */
  let sel = $derived(eq.bands[selected]);

  /** Switching type starts from that type's usual settings: a flat (0.707)
   *  Q and 24 dB/oct for cuts, a narrow Q for a notch. Gain survives only
   *  between gain types. */
  function setKind(i: number, kind: EqBandKind) {
    const b = eq.bands[i];
    if (!b || b.kind === kind) return;
    const gainDb = hasGain(kind) && hasGain(b.kind) ? b.gainDb : 0;
    const q = kind === "notch" ? 4 : hasSlope(kind) ? 0.707 : kind === "peak" || kind === "band_pass" ? 1 : 0.7;
    setBand(i, { kind, gainDb, q, slope: hasSlope(kind) ? (hasSlope(b.kind) ? b.slope : 1) : 0 });
  }
  const GRID_FREQS = [50, 100, 200, 500, 1000, 2000, 5000, 10000];

  function yForDb(db: number): number {
    return H / 2 - (db / DB_RANGE) * (H / 2 - 8);
  }
  function dbForY(y: number): number {
    return ((H / 2 - y) / (H / 2 - 8)) * DB_RANGE;
  }

  function setBand(i: number, patch: Partial<EqBand>) {
    const bands = eq.bands.map((b, j) => (j === i ? { ...b, ...patch } : b));
    onchange({ ...eq, bands, enabled: true });
  }

  function fmtFreq(f: number): string {
    return f >= 1000 ? `${(f / 1000).toFixed(f >= 10000 ? 0 : 1)}k` : `${Math.round(f)}`;
  }

  // ---- canvas drawing (redraws on any eq change via $effect) ----
  $effect(() => {
    // Reading the `eq` prop below is what makes this effect track it.
    const sel = selected;
    const ctx = canvas?.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const css = getComputedStyle(document.documentElement);
    const col = (name: string) => css.getPropertyValue(name).trim();

    ctx.fillStyle = col("--bg-0");
    ctx.fillRect(0, 0, W, H);

    // grid
    ctx.strokeStyle = "rgba(255,255,255,0.05)";
    ctx.lineWidth = 1;
    for (const f of GRID_FREQS) {
      const x = Math.round(freqToX(f, W)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, H);
      ctx.stroke();
    }
    for (const db of [-12, -6, 6, 12]) {
      const y = Math.round(yForDb(db)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
      ctx.stroke();
    }
    // 0 dB line
    ctx.strokeStyle = "rgba(255,255,255,0.14)";
    ctx.beginPath();
    ctx.moveTo(0, Math.round(yForDb(0)) + 0.5);
    ctx.lineTo(W, Math.round(yForDb(0)) + 0.5);
    ctx.stroke();

    // grid labels
    ctx.fillStyle = col("--text-3");
    ctx.font = `500 8px ${col("--mono") || "monospace"}`;
    for (const f of [100, 1000, 10000]) {
      ctx.fillText(fmtFreq(f), freqToX(f, W) + 3, H - 4);
    }

    // response curve
    const curve = responseCurve(eq.bands, 160);
    ctx.strokeStyle = eq.enabled ? col("--accent") : col("--text-3");
    ctx.lineWidth = 2;
    ctx.beginPath();
    curve.forEach((p, i) => {
      const x = freqToX(p.f, W);
      const y = yForDb(Math.max(-DB_RANGE, Math.min(DB_RANGE, p.db)));
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.stroke();

    // subtle fill under curve
    ctx.lineTo(W, yForDb(0));
    ctx.lineTo(0, yForDb(0));
    ctx.closePath();
    ctx.fillStyle = eq.enabled ? col("--accent-glow") : "rgba(255,255,255,0.03)";
    ctx.fill();

    // band handles
    eq.bands.slice(0, 4).forEach((b, i) => {
      const x = freqToX(b.freqHz, W);
      const y = yForDb(hasGain(b.kind) ? b.gainDb : 0);
      ctx.beginPath();
      ctx.arc(x, y, i === sel ? 6 : 5, 0, Math.PI * 2);
      ctx.fillStyle = col("--bg-2");
      ctx.fill();
      ctx.lineWidth = 2;
      ctx.strokeStyle = eq.enabled ? col("--accent") : col("--text-2");
      ctx.stroke();
      if (i === sel) {
        ctx.beginPath();
        ctx.arc(x, y, 9, 0, Math.PI * 2);
        ctx.strokeStyle = col("--accent-glow");
        ctx.lineWidth = 3;
        ctx.stroke();
      }
    });
  });

  // ---- interaction ----
  function bandAt(mx: number, my: number): number | null {
    let best: number | null = null;
    let bestDist = 14;
    eq.bands.slice(0, 4).forEach((b, i) => {
      const dx = freqToX(b.freqHz, W) - mx;
      const dy = yForDb(hasGain(b.kind) ? b.gainDb : 0) - my;
      const d = Math.hypot(dx, dy);
      if (d < bestDist) {
        bestDist = d;
        best = i;
      }
    });
    return best;
  }

  function canvasPos(e: PointerEvent | WheelEvent): [number, number] {
    const r = canvas.getBoundingClientRect();
    return [((e.clientX - r.left) / r.width) * W, ((e.clientY - r.top) / r.height) * H];
  }

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    const [mx, my] = canvasPos(e);
    const hit = bandAt(mx, my);
    if (hit === null) return;
    selected = hit;
    dragging = true;
    canvas.setPointerCapture(e.pointerId);
  }
  function onpointermove(e: PointerEvent) {
    if (!dragging) return;
    const [mx, my] = canvasPos(e);
    // Bands cannot cross: keep each band between its neighbors (small margin)
    // so LOW/PEAK/PEAK/HIGH always appear left-to-right in chip order.
    const bands = eq.bands;
    let lo = selected > 0 ? bands[selected - 1]!.freqHz * 1.1 : EQ_FMIN;
    let hi = selected < 3 ? bands[selected + 1]!.freqHz / 1.1 : EQ_FMAX;
    lo = Math.max(EQ_FMIN, lo);
    hi = Math.max(lo, Math.min(EQ_FMAX, hi));
    const freqHz = Math.max(lo, Math.min(hi, xToFreq(mx, W)));
    const gainDb = Math.max(-DB_RANGE, Math.min(DB_RANGE, dbForY(my)));
    // Gainless bands (cuts, notch, band-pass) only move in frequency.
    const patch: Partial<EqBand> = { freqHz: Math.round(freqHz) };
    if (hasGain(bands[selected]!.kind)) patch.gainDb = Math.round(gainDb * 10) / 10;
    setBand(selected, patch);
  }
  function onpointerup() {
    dragging = false;
  }
  function ondblclick(e: MouseEvent) {
    const [mx, my] = canvasPos(e as unknown as PointerEvent);
    const hit = bandAt(mx, my);
    if (hit !== null && hasGain(eq.bands[hit]!.kind)) setBand(hit, { gainDb: 0 });
  }
  function onwheel(e: WheelEvent) {
    e.preventDefault();
    const [mx, my] = canvasPos(e);
    const hit = bandAt(mx, my) ?? selected;
    const b = eq.bands[hit];
    if (!b || b.kind === "band_pass") return; // LSP's band-pass has a fixed width
    const dir = e.deltaY < 0 ? 1 : -1;
    const q = Math.max(0.1, Math.min(10, b.q * (dir > 0 ? 1.15 : 1 / 1.15)));
    selected = hit;
    setBand(hit, { q: Math.round(q * 100) / 100 });
  }

  function toggleEnabled() {
    onchange({ ...eq, enabled: !eq.enabled });
  }
  function close() {
    onclose();
  }
  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") close();
  }
</script>

<svelte:window {onkeydown} />

<!-- scrim click closes; Esc handled on window -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={(e) => e.target === e.currentTarget && close()}>
  <div class="panel" role="dialog" aria-label="Equalizer">
    <header>
      <div class="title">EQ <span class="sub">{title}</span></div>
      <button
        class="enbtn"
        title="Reset all bands to defaults"
        onclick={() =>
          onchange({
            ...eq,
            bands: [
              { kind: "low_shelf", freqHz: 100, gainDb: 0, q: 0.7, slope: 0 },
              { kind: "peak", freqHz: 400, gainDb: 0, q: 1, slope: 0 },
              { kind: "peak", freqHz: 2500, gainDb: 0, q: 1, slope: 0 },
              { kind: "high_shelf", freqHz: 8000, gainDb: 0, q: 0.7, slope: 0 },
              ...eq.bands.slice(4),
            ],
          })}
      >
        RESET
      </button>
      <button class="enbtn" class:active={eq.enabled} aria-pressed={eq.enabled} onclick={toggleEnabled}>
        {eq.enabled ? "ON" : "OFF"}
      </button>
      <button class="closebtn" title="Close" onclick={close}>
        <svg width="11" height="11" viewBox="0 0 12 12"><path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" stroke-width="1.25" /></svg>
      </button>
    </header>

    <canvas
      bind:this={canvas}
      style:width="{W}px"
      style:height="{H}px"
      {onpointerdown}
      {onpointermove}
      {onpointerup}
      {ondblclick}
      {onwheel}
      title="Drag dots: freq/gain (cuts, notch, band: freq only) · wheel: Q · double-click: reset gain"
    ></canvas>

    <div class="bands">
      {#each eq.bands.slice(0, 4) as b, i}
        <button class="band" class:sel={i === selected} title={KIND_TITLE[b.kind]} onclick={() => (selected = i)}>
          <span class="kind">{KIND_LABEL[b.kind]}</span>
          <span class="vals">
            {#if hasGain(b.kind)}
              {fmtFreq(b.freqHz)}Hz {b.gainDb > 0 ? "+" : ""}{b.gainDb.toFixed(1)}dB Q{b.q.toFixed(1)}
            {:else if hasSlope(b.kind)}
              {fmtFreq(b.freqHz)}Hz {SLOPES[b.slope] ?? 12}dB/oct
            {:else if b.kind === "notch"}
              {fmtFreq(b.freqHz)}Hz Q{b.q.toFixed(1)}
            {:else}
              {fmtFreq(b.freqHz)}Hz ±2 oct
            {/if}
          </span>
        </button>
      {/each}
    </div>

    {#if sel}
      <div class="opts">
        <span class="optlabel">TYPE</span>
        <div class="seg" role="radiogroup" aria-label="Band {selected + 1} type">
          {#each KINDS as k}
            <button role="radio" aria-checked={sel.kind === k} class:on={sel.kind === k} title={KIND_TITLE[k]} onclick={() => setKind(selected, k)}>
              {KIND_LABEL[k]}
            </button>
          {/each}
        </div>
      </div>
      {#if hasSlope(sel.kind)}
        <div class="opts">
          <span class="optlabel">SLOPE</span>
          <div class="seg" role="radiogroup" aria-label="Band {selected + 1} slope">
            {#each SLOPES as db, si}
              <button role="radio" aria-checked={sel.slope === si} class:on={sel.slope === si} onclick={() => setBand(selected, { slope: si })}>
                {db} dB/oct
              </button>
            {/each}
          </div>
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(10, 11, 13, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .panel {
    width: 420px;
    background: var(--bg-2);
    border: 1px solid var(--border-1);
    border-radius: 6px;
    box-shadow: inset 0 1px 0 var(--hl-top), 0 8px 24px rgba(0, 0, 0, 0.6);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title {
    flex: 1;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.08em;
  }
  .title .sub {
    color: var(--text-2);
    font-weight: 500;
    margin-left: 6px;
  }
  .enbtn {
    height: 22px;
    padding: 0 10px;
    font: 500 10px var(--mono);
    letter-spacing: 0.06em;
    background: var(--bg-3);
    color: var(--text-3);
    border: 1px solid var(--border-0);
    border-radius: 2px;
    box-shadow: inset 0 1px 0 var(--hl-top);
  }
  .enbtn.active {
    background: var(--accent-glow);
    color: var(--accent);
    border-color: var(--accent);
  }
  .closebtn {
    width: 24px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-2);
    border-radius: 2px;
  }
  .closebtn:hover {
    background: var(--mute);
    color: #fff;
  }
  canvas {
    display: block;
    border: 1px solid var(--border-0);
    border-radius: 2px;
    cursor: crosshair;
    touch-action: none;
  }
  .bands {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
  }
  .band {
    display: flex;
    flex-direction: column;
    gap: 2px;
    align-items: center;
    padding: 5px 2px;
    background: var(--bg-3);
    border: 1px solid var(--border-0);
    border-radius: 2px;
    box-shadow: inset 0 1px 0 var(--hl-top);
  }
  .band:hover {
    background: var(--bg-4);
  }
  .band.sel {
    border-color: var(--accent);
  }
  .kind {
    font-size: 9px;
    letter-spacing: 0.08em;
    color: var(--text-2);
  }
  .band.sel .kind {
    color: var(--accent);
  }
  .vals {
    font: 500 9px var(--mono);
    color: var(--text-1);
    white-space: nowrap;
  }
  .opts {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .optlabel {
    width: 38px;
    flex: none;
    font: 500 9px var(--mono);
    letter-spacing: 0.08em;
    color: var(--text-3);
  }
  .seg {
    flex: 1;
    display: flex;
    gap: 2px;
  }
  .seg button {
    flex: 1;
    height: 20px;
    padding: 0 2px;
    font: 500 9px var(--mono);
    letter-spacing: 0.04em;
    white-space: nowrap;
    color: var(--text-3);
    background: var(--bg-3);
    border: 1px solid var(--border-0);
    border-radius: 2px;
    box-shadow: inset 0 1px 0 var(--hl-top);
  }
  .seg button:hover {
    background: var(--bg-4);
    color: var(--text-2);
  }
  .seg button.on {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-glow);
  }
</style>
