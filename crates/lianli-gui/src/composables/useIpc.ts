import { invoke } from "@tauri-apps/api/core";
import type { PollResult } from "@/types";

/**
 * Thin wrapper around the Rust `ipc_request` command.
 *
 * Every daemon method is forwarded as `{"method": <method>, "params": <params>}`
 * over the Unix socket. **No-arg methods must send `null` params**, not `{}` —
 * the daemon's `IpcRequest` is an internally-tagged enum, and serde rejects a
 * JSON object for a unit variant (e.g. `GetConfig`) while `null` is accepted.
 */
export function useIpc() {
  return {
    async request<T = unknown>(method: string, params?: object | null): Promise<T> {
      return invoke<T>("ipc_request", {
        method,
        params: params ?? null,
      });
    },
    async poll(): Promise<PollResult> {
      return invoke<PollResult>("poll_daemon");
    },
    async connectionInfo(): Promise<[boolean, string]> {
      return invoke<[boolean, string]>("connection_info");
    },
    async openEditorWindow(templateId?: string): Promise<void> {
      return invoke<void>("open_editor_window", {
        templateId: templateId ?? null,
      });
    },
    async openBrowserWindow(): Promise<void> {
      return invoke<void>("open_browser_window");
    },
  };
}

