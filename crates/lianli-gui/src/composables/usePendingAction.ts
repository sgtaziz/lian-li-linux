import { reactive } from "vue";
import type { PendingActionKind } from "@/types";

const PENDING_TIMEOUT_MS = 10_000;

interface PendingEntry {
  kind: PendingActionKind;
  startedAt: number;
}

/**
 * Tracks in-flight device actions (bind/unbind/display-mode switch/fan-qty).
 *
 * An entry is cleared when the daemon reports the expected state change or
 * after the 10s safety timeout — whichever comes first. Mirrors the Slint
 * GUI's `SharedState::pending_actions` semantics.
 */
export function usePendingAction() {
  const pending = reactive<Record<string, PendingEntry>>({});

  function set(deviceId: string, kind: PendingActionKind) {
    pending[deviceId] = { kind, startedAt: Date.now() };
  }

  function clear(deviceId: string) {
    delete pending[deviceId];
  }

  function get(deviceId: string): PendingActionKind | null {
    return pending[deviceId]?.kind ?? null;
  }

  /** Expire timed-out entries and reconcile against the current device list. */
  function expire(deviceIds: string[]) {
    const now = Date.now();
    const present = new Set(deviceIds);
    for (const [key, entry] of Object.entries(pending)) {
      if (now - entry.startedAt >= PENDING_TIMEOUT_MS) {
        delete pending[key];
        continue;
      }
      switch (entry.kind) {
        case "bind": {
          // key is "wireless-unbound:<mac>"; clears once "wireless:<mac>" appears
          const mac = key.startsWith("wireless-unbound:")
            ? key.slice("wireless-unbound:".length)
            : null;
          const boundId = mac ? `wireless:${mac}` : null;
          if (boundId && present.has(boundId)) delete pending[key];
          break;
        }
        case "unbind":
        case "switch":
          // clears once the device reappears (post-switch) — kept until then
          if (present.has(key)) delete pending[key];
          break;
        case "fan-quantity":
          // cleared immediately on next poll (one-shot)
          delete pending[key];
          break;
      }
    }
  }

  return { pending, set, clear, get, expire };
}
