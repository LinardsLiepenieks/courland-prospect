import { defineManifest } from "@crxjs/vite-plugin";

// The pinned public key fixes the extension ID to
// `knipphmpmemfkimdiknnjjbelecnkenf`, regardless of the load path. The Rust
// backend hard-codes that ID for its CORS allowlist, so this key must never
// change. (The private key lives in .keys/, gitignored.)
const KEY =
  "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA0nNneBsJGPzTcrLahdjhFa5D9vkynAl8il2XGTesKuTQsRo337uSDKsZMnSLQHRP3hBLNqE3BcBtnrzaS6mhMB9lXCQnyZwM7byFk8Mr19QkMOM68n1gfrbhchq3fEaREib3vtAQejY0DLtvP7Eh8PENez+UBfjFM2aCMAfI8+doDuXHPE99ZMa6he2vdPZvJJ+JIKUW6nK45AfybXTEgHaOiqcgvcxLKfTj9VVf29RgEnWlouxEwn4l892WvunOsN/LLowbk0y6WdOAYktScn2CW3/r+jtUrVKe425rhhhYkfj/lpB5gwC+EoRb8dn0ClqpmtydBXuOUX5O0RgepwIDAQAB";

export default defineManifest({
  manifest_version: 3,
  name: "Courland Prospect — LinkedIn Capture",
  description:
    "Adds an 'Add to Prospects' button to LinkedIn chats, feeding your Courland CRM.",
  version: "0.1.0",
  key: KEY,
  minimum_chrome_version: "116",
  // storage: last-used pitch + the message outbox. alarms: periodic outbox flush
  // (delivers capture done while the app was closed). clipboardRead/Write: the
  // comment scrape captures each post's permalink the only reliable way LinkedIn
  // exposes it — its ⋯ "Copy link to post" writes the URL to the clipboard, which
  // the (foreground) scrape tab then reads back. No broad permissions.
  permissions: ["storage", "alarms", "clipboardRead", "clipboardWrite"],
  host_permissions: [
    "https://www.linkedin.com/*",
    // The loopback ingest server (any port on 127.0.0.1 — match patterns ignore
    // the port). Required for the service worker's cross-origin fetch.
    "http://127.0.0.1/*",
  ],
  background: {
    service_worker: "src/background.ts",
    type: "module",
  },
  content_scripts: [
    {
      matches: ["https://www.linkedin.com/*"],
      js: ["src/content/main.ts"],
      run_at: "document_idle",
    },
  ],
});
