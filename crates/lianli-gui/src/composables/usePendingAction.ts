import { reactive } from "vue";
import type { PendingActionKind } from "@/types";

const PENDING_TIMEOUT_MS = 10_000;

interface PendingEntry {
  kind: PendingActionKind;
  startedAt: number;
  awaitingResult: boolean;
}

export function usePendingAction() {
  const pending = reactive<Record<string, PendingEntry>>({});

  function set(deviceId: string, kind: PendingActionKind, awaitingResult = false) {
    pending[deviceId] = { kind, startedAt: Date.now(), awaitingResult };
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
      if (entry.awaitingResult) continue;
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
          if (!present.has(key)) delete pending[key];
          break;
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
