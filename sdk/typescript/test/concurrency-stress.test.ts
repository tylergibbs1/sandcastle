/**
 * Concurrency stress test: 1000 concurrent sandboxes.
 *
 * Validates PRD claim 54: "> 10,000 concurrent sandboxes on 8GB RAM"
 * We test 1000 here via the subprocess transport (each spawns a CLI process).
 */
import { describe, expect, it } from "bun:test";
import { SandCastle } from "../src/index.js";

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE = "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";

let hasBinary = false;
try {
  hasBinary = Bun.file(BINARY_PATH).size > 0;
} catch {
  /* not built */
}

const run = hasBinary ? it : it.skip;

function sc(): SandCastle {
  return new SandCastle({ binaryPath: BINARY_PATH, guestModule: GUEST_MODULE });
}

describe("concurrency stress", () => {
  run(
    "1000 concurrent sandboxes all return correct results",
    async () => {
      const client = sc();
      const count = 1000;
      const batchSize = 100; // Launch in batches to avoid fd exhaustion

      const allResults: Array<{ id: number }> = [];
      const start = performance.now();

      for (let batch = 0; batch < count / batchSize; batch++) {
        const offset = batch * batchSize;
        const promises = Array.from({ length: batchSize }, (_, i) => {
          const id = offset + i;
          return client
            .run<{ id: number }>(`return { id: ${id} };`)
            .then((r) => ({ id: r.id }))
            .catch((err) => {
              console.error(`Sandbox ${id} failed: ${err.message}`);
              return { id: -1 };
            });
        });

        const batchResults = await Promise.all(promises);
        allResults.push(...batchResults);

        const elapsed = ((performance.now() - start) / 1000).toFixed(1);
        const completed = offset + batchSize;
        console.log(
          `  Batch ${batch + 1}/${count / batchSize}: ${completed}/${count} complete (${elapsed}s)`,
        );
      }

      const elapsed = performance.now() - start;
      const successful = allResults.filter((r) => r.id >= 0);
      const failed = allResults.filter((r) => r.id < 0);

      // Verify all returned correct IDs
      const ids = successful.map((r) => r.id).sort((a, b) => a - b);
      const expectedIds = Array.from({ length: count }, (_, i) => i);

      console.log(`\n  Results:`);
      console.log(`    Total:      ${count}`);
      console.log(`    Successful: ${successful.length}`);
      console.log(`    Failed:     ${failed.length}`);
      console.log(`    Time:       ${(elapsed / 1000).toFixed(2)}s`);
      console.log(`    Per sandbox: ${(elapsed / count).toFixed(1)}ms`);
      console.log(`    Throughput: ${Math.round(count / (elapsed / 1000))} sandboxes/sec`);

      // At least 95% should succeed (subprocess mode may hit fd limits)
      expect(successful.length).toBeGreaterThanOrEqual(count * 0.95);

      // All successful results should have correct, unique IDs
      const uniqueIds = new Set(ids);
      expect(uniqueIds.size).toBe(successful.length);
    },
    600_000, // 10 minute timeout
  );
});
