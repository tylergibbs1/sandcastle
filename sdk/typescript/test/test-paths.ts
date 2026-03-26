import { fileURLToPath } from "node:url";

export const BINARY_PATH = fileURLToPath(new URL("../../../target/release/sandcastle", import.meta.url));
export const GUEST_MODULE = fileURLToPath(
  new URL("../../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm", import.meta.url),
);
export const ENV_FILE = fileURLToPath(new URL("../../../.env", import.meta.url));
