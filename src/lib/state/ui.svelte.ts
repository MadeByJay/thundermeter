/** Transient UI state that isn't mixer state (open panels, etc.). */
import type { BusId } from "../types";

class UiStore {
  /** Strip id whose EQ panel is open, or null. */
  eqStrip = $state<number | null>(null);
  /** Bus whose EQ panel is open, or null. At most one of eqStrip/eqBus is set. */
  eqBus = $state<BusId | null>(null);
  /** Strip id whose Dynamics (gate/comp) panel is open, or null. */
  dynStrip = $state<number | null>(null);
}

export const ui = new UiStore();
