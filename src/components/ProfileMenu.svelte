<script lang="ts">
  import { tick } from "svelte";
  import { mixer } from "../lib/state/mixer.svelte";
  import { profileNameProblem } from "../lib/utils/profile";

  /** "save" = save the current mixer as a new profile; "rename" = rename the active one. */
  type Mode = { kind: "list" } | { kind: "save" | "rename"; draft: string } | { kind: "delete"; name: string };

  let open = $state(false);
  let mode = $state<Mode>({ kind: "list" });
  let root: HTMLDivElement;
  let input = $state<HTMLInputElement>();

  let problem = $derived.by(() => {
    if (mode.kind !== "save" && mode.kind !== "rename") return null;
    // Renaming to the same name is a no-op, not a clash.
    const taken = mode.kind === "rename" ? mixer.profiles.filter((p) => p !== mixer.profileName) : mixer.profiles;
    return profileNameProblem(mode.draft, taken);
  });

  function toggle() {
    open = !open;
    mode = { kind: "list" };
  }
  function close() {
    open = false;
    mode = { kind: "list" };
  }
  async function startEdit(kind: "save" | "rename") {
    mode = { kind, draft: kind === "rename" ? mixer.profileName : "" };
    await tick();
    input?.focus();
    input?.select();
  }
  function commitEdit() {
    if (problem || (mode.kind !== "save" && mode.kind !== "rename")) return;
    if (mode.kind === "save") mixer.saveProfileAs(mode.draft);
    else if (mode.draft !== mixer.profileName) mixer.renameProfile(mixer.profileName, mode.draft);
    close();
  }
  function choose(name: string) {
    mixer.loadProfile(name);
    close();
  }

  function onwindowpointerdown(e: PointerEvent) {
    if (open && !root.contains(e.target as Node)) close();
  }
  function onwindowkeydown(e: KeyboardEvent) {
    if (!open || e.key !== "Escape") return;
    if (mode.kind === "list") close();
    else mode = { kind: "list" };
  }
</script>

<svelte:window onpointerdown={onwindowpointerdown} onkeydown={onwindowkeydown} />

<div class="wrap" bind:this={root} data-no-drag>
  <button class="chip" class:open title="Profiles" aria-haspopup="menu" aria-expanded={open} onclick={toggle}>
    {mixer.profileName}
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
      <path d="M2.5 4l2.5 2.5L7.5 4" stroke="currentColor" stroke-width="1.25" />
    </svg>
  </button>

  {#if open}
    <div class="menu" role="menu" aria-label="Profiles">
      <div class="head">PROFILES</div>
      <ul>
        {#each mixer.profiles as name (name)}
          {@const active = name === mixer.profileName}
          <li class:active>
            {#if mode.kind === "delete" && mode.name === name}
              <span class="confirm">Delete “{name}”?</span>
              <button class="danger" onclick={() => { mixer.deleteProfile(name); mode = { kind: "list" }; }}>DELETE</button>
              <button class="ghost" onclick={() => (mode = { kind: "list" })}>KEEP</button>
            {:else}
              <button class="row" role="menuitemradio" aria-checked={active} onclick={() => choose(name)}>
                <span class="dot" aria-hidden="true"></span>
                <span class="name">{name}</span>
              </button>
              {#if !active}
                <button class="trash" title="Delete {name}" aria-label="Delete {name}" onclick={() => (mode = { kind: "delete", name })}>
                  <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
                    <path d="M2.5 3.5h7M5 3.5V2.5h2v1M3.5 3.5l.5 6h4l.5-6" stroke="currentColor" stroke-width="1.1" />
                  </svg>
                </button>
              {/if}
            {/if}
          </li>
        {/each}
      </ul>

      <div class="sep"></div>
      {#if mode.kind === "save" || mode.kind === "rename"}
        <form class="edit" onsubmit={(e) => { e.preventDefault(); commitEdit(); }}>
          <label for="profile-name">{mode.kind === "save" ? "Save current mixer as" : `Rename “${mixer.profileName}” to`}</label>
          <div class="field">
            <input id="profile-name" bind:this={input} bind:value={mode.draft} maxlength="64" autocomplete="off" spellcheck="false" />
            <button class="primary" type="submit" disabled={!!problem}>{mode.kind === "save" ? "SAVE" : "RENAME"}</button>
          </div>
          <div class="hint" class:bad={!!problem && mode.draft !== ""}>{problem ?? "Enter to confirm · Esc to go back"}</div>
        </form>
      {:else}
        <button class="action" onclick={() => startEdit("save")}>Save as new profile…</button>
        <button class="action" onclick={() => startEdit("rename")}>Rename “{mixer.profileName}”…</button>
      {/if}
      <p class="note">Switching keeps audio playing when both profiles use the same strips and devices. Otherwise the mixer rebuilds, and apps may need play pressed again.</p>
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
  }
  .chip {
    height: 24px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--bg-3);
    border: 1px solid var(--border-0);
    border-radius: 2px;
    box-shadow: inset 0 1px 0 var(--hl-top);
    font-size: 12px;
    color: var(--text-1);
  }
  .chip:hover,
  .chip.open {
    background: var(--bg-4);
  }
  .chip svg {
    color: var(--text-3);
  }
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 50%;
    transform: translateX(-50%);
    z-index: 50;
    width: 260px;
    padding: 8px;
    background: var(--bg-2);
    border: 1px solid var(--border-1);
    border-radius: 4px;
    box-shadow: inset 0 1px 0 var(--hl-top), 0 8px 24px rgba(0, 0, 0, 0.6);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .head {
    font: 500 9px var(--mono);
    letter-spacing: 0.1em;
    color: var(--text-3);
    padding: 2px 4px 4px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 240px;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    gap: 4px;
    min-height: 26px;
    border-radius: 2px;
  }
  li:hover {
    background: var(--bg-3);
  }
  .row {
    flex: 1;
    min-width: 0;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 6px;
    text-align: left;
    font-size: 12px;
    color: var(--text-2);
  }
  li.active .row {
    color: var(--text-1);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    border: 1px solid var(--border-1);
    flex: none;
  }
  li.active .dot {
    background: var(--accent);
    border-color: var(--accent);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trash {
    width: 24px;
    height: 22px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-3);
    border-radius: 2px;
    opacity: 0;
  }
  li:hover .trash,
  .trash:focus-visible {
    opacity: 1;
  }
  .trash:hover {
    color: var(--mute);
  }
  .confirm {
    flex: 1;
    min-width: 0;
    padding: 0 6px;
    font-size: 11px;
    color: var(--text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .danger,
  .ghost,
  .primary {
    height: 22px;
    padding: 0 8px;
    font: 500 9px var(--mono);
    letter-spacing: 0.06em;
    border-radius: 2px;
    border: 1px solid var(--border-0);
    background: var(--bg-3);
    color: var(--text-2);
    flex: none;
  }
  .danger {
    color: var(--mute);
    border-color: var(--mute);
  }
  .primary {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-glow);
  }
  .primary:disabled {
    color: var(--text-3);
    border-color: var(--border-0);
    background: var(--bg-3);
  }
  .sep {
    height: 1px;
    background: var(--border-0);
    margin: 2px -8px;
  }
  .action {
    height: 26px;
    padding: 0 6px;
    text-align: left;
    font-size: 12px;
    color: var(--text-2);
    border-radius: 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .action:hover {
    background: var(--bg-3);
    color: var(--text-1);
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 2px 2px 0;
  }
  .edit label {
    font-size: 11px;
    color: var(--text-2);
  }
  .field {
    display: flex;
    gap: 4px;
  }
  .field input {
    flex: 1;
    min-width: 0;
    height: 22px;
    padding: 0 6px;
    font-size: 12px;
    color: var(--text-1);
    background: var(--bg-0);
    border: 1px solid var(--border-1);
    border-radius: 2px;
    outline: none;
  }
  .field input:focus {
    border-color: var(--accent);
  }
  .hint {
    font-size: 10px;
    color: var(--text-3);
  }
  .hint.bad {
    color: var(--mute);
  }
  .note {
    margin: 2px 4px 0;
    font-size: 10px;
    line-height: 1.4;
    color: var(--text-3);
  }
</style>
