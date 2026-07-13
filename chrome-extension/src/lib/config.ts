// Reads the app-written config.json (token + port). This file lives in the
// extension's own package and is fetched via getURL — it is NOT a
// web_accessible_resource, so only the extension can read it, keeping the token
// private. The Courland app writes it into the load-unpacked folder at startup.

export interface ExtConfig {
  token: string;
  appPort: number;
}

let cached: ExtConfig | null = null;

export async function getConfig(): Promise<ExtConfig> {
  if (cached) return cached;
  const res = await fetch(chrome.runtime.getURL("config.json"));
  if (!res.ok) {
    throw new Error(
      "config.json missing — open the Courland app so it can provision the extension.",
    );
  }
  const parsed = (await res.json()) as Partial<ExtConfig>;
  if (!parsed.token || !parsed.appPort) {
    throw new Error("config.json is malformed (missing token or appPort).");
  }
  cached = { token: parsed.token, appPort: parsed.appPort };
  return cached;
}

/** Drop the cached config so the next `getConfig()` re-reads config.json. The
 *  app rewrites that file (fetched live from the unpacked extension) when it
 *  restarts on a different port or rotates the token; without this, a
 *  long-lived service worker would keep hitting the stale port/token until it's
 *  evicted. Callers invalidate after a transport failure, then retry. */
export function invalidateConfig(): void {
  cached = null;
}

export function baseUrl(cfg: ExtConfig): string {
  return `http://127.0.0.1:${cfg.appPort}`;
}
