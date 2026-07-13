import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * The hard-gate status, mirrored from the Rust `GateStatus` enum
 * (serde tag = "state", content = "detail").
 */
export type GateStatus =
  | { state: "initializing" }
  | { state: "chromeClosed" }
  | { state: "extensionMissing" }
  | { state: "ready" }
  | { state: "error"; detail: string };

export function gateStatus(): Promise<GateStatus> {
  return invoke("gate_status");
}

/** The writable extension folder to load-unpack from. */
export function extensionDir(): Promise<string> {
  return invoke("extension_dir");
}

/** Subscribe to gate transitions pushed from the backend. */
export function onGateStatus(cb: (status: GateStatus) => void): Promise<UnlistenFn> {
  return listen<GateStatus>("gate://status", (event) => cb(event.payload));
}
