const FAILURE_EXIT_CODE = 1;
const UINT32_RANGE = 4_294_967_296;

const createRandom = (seed) => {
  let state = seed >>> 0;

  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / UINT32_RANGE;
  };
};

/** Create repeatable zero-based lookup positions within exclusive bounds. */
export const createDeterministicLookups = (count, maxLine, maxColumn, seed) => {
  const random = createRandom(seed);
  const lookups = [];

  for (let index = 0; index < count; index++) {
    lookups.push({
      line: Math.floor(random() * maxLine),
      column: Math.floor(random() * maxColumn),
    });
  }

  return lookups;
};

/** Mark the benchmark process as failed when any implementation mismatches. */
export const setFailureExitCode = (results) => {
  const failed = results.some(({ wasmPass, napiPass }) => !wasmPass || !napiPass);

  if (failed) {
    process.exitCode = FAILURE_EXIT_CODE;
  }
};
